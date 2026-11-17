//! Resolve exact kernel debug packages without changing the host's APT setup.
use crate::{
    sources::USER_AGENT,
    ui::{self, user_error},
};
use anyhow::{Context, Result, bail};
use reqwest::blocking::Client;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Component, Path};
use std::process::{Command, Stdio};
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Package {
    pub url: String,
    pub version: String,
    pub sha256: Option<String>,
    pub format: PackageFormat,
}

#[derive(Clone, Copy, Debug)]
pub enum PackageFormat {
    Deb,
    Rpm,
    Apk,
}

pub fn repository_arch<'a>(distro: &str, arch: &'a str) -> &'a str {
    if distro == "fedora" {
        match arch {
            "amd64" => "x86_64",
            "arm64" => "aarch64",
            "i386" => "i686",
            "ppc64el" => "ppc64le",
            "armhf" => "armv7",
            _ => arch,
        }
    } else {
        arch
    }
}

pub fn resolve_fedora(
    client: &Client,
    kernel: &str,
    arch: &str,
    release: Option<&str>,
    version: Option<&str>,
) -> Result<Package> {
    let arch = repository_arch("fedora", arch);
    let build = kernel.strip_suffix(&format!(".{arch}"))
        .context("Fedora --kernel must be the full uname -r, including architecture (for example 6.8.5-301.fc40.x86_64)")?;
    let (upstream, revision) = build
        .split_once('-')
        .context("Fedora kernel lacks a build revision")?;
    if upstream.is_empty() || revision.is_empty() || !revision.contains(".fc") {
        bail!("Expected a Fedora kernel build such as 6.8.5-301.fc40.x86_64");
    }
    if let Some(release) = release
        && !revision.ends_with(&format!(".fc{release}"))
    {
        bail!("Fedora release {release} does not match kernel {kernel}");
    }
    if version.is_some_and(|v| v != build) {
        bail!("--package-version must match the Fedora kernel build {build}");
    }
    let url = format!(
        "https://kojipkgs.fedoraproject.org/packages/kernel/{upstream}/{revision}/{arch}/kernel-debuginfo-{build}.{arch}.rpm"
    );
    let status = client
        .head(&url)
        .timeout(Duration::from_secs(60))
        .send()
        .with_context(|| format!("Cannot reach {url}"))?
        .status();
    if !status.is_success() {
        return Err(user_error(
            format!("No Fedora debug package for {kernel} (HTTP {status})"),
            Some("--package-url <url> or --vmlinux <path>"),
        ));
    }
    Ok(Package {
        url,
        version: build.into(),
        sha256: None,
        format: PackageFormat::Rpm,
    })
}

pub fn client() -> Result<Client> {
    Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(3600))
        .build()
        .context("Cannot create download client")
}

pub fn package_names(distro: &str, kernel: &str) -> Vec<String> {
    match distro {
        "ubuntu" => vec![
            format!("linux-image-unsigned-{kernel}-dbgsym"),
            format!("linux-image-{kernel}-dbgsym"),
            format!("linux-image-{kernel}-dbg"),
        ],
        "debian" => vec![
            format!("linux-image-{kernel}-dbg"),
            format!("linux-image-{kernel}-dbgsym"),
            format!("linux-image-{kernel}-unsigned-dbg"),
        ],
        _ => Vec::new(),
    }
}

fn repositories(distro: &str, release: &str) -> Vec<(&'static str, String)> {
    match distro {
        "ubuntu" => ["-updates", "", "-proposed"]
            .iter()
            .map(|suffix| ("https://ddebs.ubuntu.com/", format!("{release}{suffix}")))
            .collect(),
        "debian" => vec![
            ("https://deb.debian.org/debian/", release.to_string()),
            (
                "https://deb.debian.org/debian/",
                format!("{release}-updates"),
            ),
            (
                "https://deb.debian.org/debian/",
                format!("{release}-backports"),
            ),
            (
                "https://deb.debian.org/debian-security/",
                format!("{release}-security"),
            ),
        ],
        _ => Vec::new(),
    }
}

/// Never guess a package filename or substitute a nearby kernel. Multiple build
/// revisions for the same ABI require an explicit --package-version.
pub fn resolve(
    client: &Client,
    distro: &str,
    release: &str,
    kernel: &str,
    arch: &str,
    version: Option<&str>,
    workspace: &Path,
) -> Result<Package> {
    let names = package_names(distro, kernel);
    let mut found = Vec::new();
    let mut failures = Vec::new();
    let repositories = repositories(distro, release);
    let total = repositories.len();
    for (base, suite) in repositories {
        let url = format!("{base}dists/{suite}/main/binary-{arch}/Packages.xz");
        let result = (|| -> Result<Vec<Package>> {
            let mut response = client
                .get(&url)
                .timeout(Duration::from_secs(120))
                .send()
                .with_context(|| format!("{suite} index unreachable"))?;
            if !response.status().is_success() {
                bail!("{suite} index returned HTTP {}", response.status());
            }
            let archive = workspace.join("Packages.xz");
            let mut file = File::create(&archive)?;
            std::io::copy(&mut response, &mut file)?;
            drop(file);
            let unpacked = workspace.join("Packages");
            let output = Command::new("xz")
                .args(["-dc", "--"])
                .arg(&archive)
                .stdout(Stdio::from(File::create(&unpacked)?))
                .output()
                .context("Cannot run xz; install xz-utils (or xz)")?;
            if !output.status.success() {
                bail!(
                    "xz failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                );
            }
            parse_index(
                BufReader::new(File::open(unpacked)?),
                base,
                &names,
                arch,
                version,
            )
        })();
        match result {
            Ok(packages) => found.extend(packages),
            Err(error) => failures.push(format!("{error:#}")),
        }
    }
    prefer_debug_payloads(&mut found, &names);
    match found.len() {
        // Every suite failing usually means a wrong codename or no network,
        // not a missing kernel, so report the first cause instead.
        0 if failures.len() == total => Err(user_error(
            format!("Cannot read {distro} {release} package indexes"),
            failures.first().map(String::as_str),
        )),
        0 => Err(user_error(
            format!(
                "No debug package for {distro} {release} {kernel} {arch}{}",
                version.map(|v| format!(" {v}")).unwrap_or_default()
            ),
            Some("--package-url <url> or --vmlinux <path>"),
        )),
        1 => Ok(found.remove(0)),
        _ => Err(user_error(
            format!("Several builds match {kernel}"),
            Some(&format!(
                "--package-version {}",
                found
                    .iter()
                    .map(|p| p.version.as_str())
                    .collect::<Vec<_>>()
                    .join(" | ")
            )),
        )),
    }
}

fn prefer_debug_payloads(found: &mut Vec<Package>, names: &[String]) {
    let rank = |package: &Package| {
        let file = package.url.rsplit('/').next().unwrap_or_default();
        names
            .iter()
            .position(|name| file.starts_with(&format!("{name}_")))
            .unwrap_or(usize::MAX)
    };
    // Ubuntu's signed kernel debug package can be a tiny dependency-only alias.
    // Prefer the unsigned debug payload before deduplicating each build version.
    found.sort_by(|a, b| {
        a.version
            .cmp(&b.version)
            .then_with(|| rank(a).cmp(&rank(b)))
            .then_with(|| a.url.cmp(&b.url))
    });
    found.dedup_by(|a, b| a.version == b.version);
}

fn parse_index(
    reader: impl BufRead,
    base: &str,
    names: &[String],
    arch: &str,
    version: Option<&str>,
) -> Result<Vec<Package>> {
    let mut found = Vec::new();
    let mut fields = HashMap::new();
    for line in reader.lines().chain(std::iter::once(Ok(String::new()))) {
        let line = line.context("Cannot read package index")?;
        if line.is_empty() {
            if names.iter().any(|name| fields.get("Package") == Some(name))
                && fields.get("Architecture").map(String::as_str) == Some(arch)
                && version.is_none_or(|v| fields.get("Version").map(String::as_str) == Some(v))
            {
                let filename = fields
                    .get("Filename")
                    .context("Package index lacks Filename")?;
                if !filename.starts_with("pool/")
                    || Path::new(filename)
                        .components()
                        .any(|c| !matches!(c, Component::Normal(_)))
                {
                    bail!("Invalid package path in index: {filename}");
                }
                let hash = fields.get("SHA256").context("Package index lacks SHA256")?;
                if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
                    bail!("Invalid package SHA256 in index");
                }
                found.push(Package {
                    url: format!("{base}{filename}"),
                    version: fields
                        .get("Version")
                        .context("Package index lacks Version")?
                        .clone(),
                    sha256: Some(hash.to_ascii_lowercase()),
                    format: PackageFormat::Deb,
                });
            }
            fields.clear();
        } else if !line.starts_with([' ', '\t'])
            && let Some((key, value)) = line.split_once(": ")
            && matches!(
                key,
                "Package" | "Version" | "Architecture" | "Filename" | "SHA256"
            )
        {
            fields.insert(key.to_string(), value.to_string());
        }
    }
    Ok(found)
}

/// Stream large kernel packages to disk and compare repository checksums.
pub fn download(client: &Client, package: &Package, dest: &Path) -> Result<()> {
    let mut response = client
        .get(&package.url)
        .send()
        .with_context(|| format!("Cannot download {}", package.url))?
        .error_for_status()?;
    let bar = ui::download_bar(response.content_length());
    let result = (|| -> Result<()> {
        let mut file = File::create(dest)?;
        let mut hasher = Sha256::new();
        let mut buffer = [0u8; 65536];
        loop {
            let count = response
                .read(&mut buffer)
                .context("Kernel package download interrupted")?;
            if count == 0 {
                break;
            }
            file.write_all(&buffer[..count])?;
            hasher.update(&buffer[..count]);
            bar.inc(count as u64);
        }
        if let Some(expected) = &package.sha256 {
            let actual = format!("{:x}", hasher.finalize());
            if &actual != expected {
                bail!("Kernel package SHA256 mismatch: expected {expected}, got {actual}");
            }
        }
        Ok(())
    })();
    bar.finish_and_clear();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn stanza(name: &str, arch: &str, version: &str) -> String {
        format!(
            "Package: {name}\nArchitecture: {arch}\nVersion: {version}\nFilename: pool/main/l/linux/kernel.ddeb\nSHA256: {}\nDescription: Debug info\n continuation\n",
            "a".repeat(64)
        )
    }

    #[test]
    fn matches_exact_kernel_architecture_and_package_version_at_eof() {
        let names = package_names("ubuntu", "6.8.0-31-generic");
        let input = [
            stanza(&names[0], "arm64", "6.8.0-31.31"),
            stanza(
                "linux-image-unsigned-6.8.0-32-generic-dbgsym",
                "amd64",
                "6.8.0-32.32",
            ),
            stanza(&names[0], "amd64", "6.8.0-31.32"),
            stanza(&names[0], "amd64", "6.8.0-31.31"),
        ]
        .join("\n");
        let packages = parse_index(
            Cursor::new(input),
            "https://example.org/",
            &names,
            "amd64",
            Some("6.8.0-31.31"),
        )
        .unwrap();
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].version, "6.8.0-31.31");
        assert_eq!(
            packages[0].url,
            "https://example.org/pool/main/l/linux/kernel.ddeb"
        );
    }

    #[test]
    fn rejects_missing_checksums_and_escaping_paths() {
        let names = package_names("debian", "6.1.0-28-amd64");
        let input = stanza(&names[0], "amd64", "6.1.119-1");
        for bad in [
            input.replace("pool/main/l/linux/kernel.ddeb", "pool/../../oops"),
            input.replace(&format!("SHA256: {}\n", "a".repeat(64)), ""),
        ] {
            assert!(
                parse_index(
                    Cursor::new(bad),
                    "https://example.org/",
                    &names,
                    "amd64",
                    None
                )
                .is_err()
            );
        }
    }

    #[test]
    fn prefers_unsigned_payload_over_signed_alias_without_hiding_other_builds() {
        let names = package_names("ubuntu", "6.8.0-31-generic");
        let package = |name: &str, version: &str| Package {
            url: format!("https://example.org/{name}_{version}_amd64.ddeb"),
            version: version.into(),
            sha256: None,
            format: PackageFormat::Deb,
        };
        let mut found = vec![
            package(&names[1], "6.8.0-31.31"),
            package(&names[0], "6.8.0-31.31"),
            package(&names[0], "6.8.0-31.32"),
        ];
        prefer_debug_payloads(&mut found, &names);
        assert_eq!(found.len(), 2);
        assert!(found[0].url.contains(&names[0]));
        assert_eq!(found[1].version, "6.8.0-31.32");
    }

    #[test]
    fn fedora_requires_matching_full_kernel_release_before_network_access() {
        let client = client().unwrap();
        assert!(resolve_fedora(&client, "6.8.5-301.fc40", "amd64", None, None).is_err());
        assert!(resolve_fedora(&client, "6.8.5-301.fc40.x86_64", "arm64", None, None).is_err());
        assert!(
            resolve_fedora(&client, "6.8.5-301.fc40.x86_64", "amd64", Some("41"), None).is_err()
        );
        assert!(
            resolve_fedora(
                &client,
                "6.8.5-301.fc40.x86_64",
                "amd64",
                Some("40"),
                Some("6.8.5-302.fc40")
            )
            .is_err()
        );
    }

    #[test]
    #[ignore = "downloads official package indexes; run explicitly with network access"]
    fn live_repository_lookup() {
        let client = client().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let ubuntu = resolve(
            &client,
            "ubuntu",
            "noble",
            "6.8.0-31-generic",
            "amd64",
            Some("6.8.0-31.31"),
            workspace.path(),
        )
        .unwrap();
        assert!(
            ubuntu
                .url
                .contains("linux-image-unsigned-6.8.0-31-generic-dbgsym_")
        );
        assert!(ubuntu.sha256.is_some());
        let debian = resolve(
            &client,
            "debian",
            "bookworm",
            "6.1.0-47-amd64",
            "amd64",
            None,
            workspace.path(),
        )
        .unwrap();
        assert!(debian.url.contains("linux-image-6.1.0-47-amd64-dbg_"));
        let fedora =
            resolve_fedora(&client, "6.8.5-301.fc40.x86_64", "amd64", Some("40"), None).unwrap();
        assert!(
            fedora
                .url
                .ends_with("/kernel-debuginfo-6.8.5-301.fc40.x86_64.rpm")
        );
    }
}
