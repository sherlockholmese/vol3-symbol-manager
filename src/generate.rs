use crate::packages::{self, Package, PackageFormat};
use crate::symbols::find_symbols_dir;
use crate::ui::{self, muted, success, user_error};
use anyhow::{Context, Result, bail};
use base64::Engine;
use clap::Args;
use serde::Deserialize;
use serde::de::{IgnoredAny, MapAccess, Visitor};
use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tempfile::NamedTempFile;

#[derive(Args, Debug)]
pub struct GenerateArgs {
    /// ubuntu, debian or fedora
    #[arg(long, value_parser = token)]
    pub distro: String,
    /// Target kernel release, as printed by uname -r
    #[arg(long, value_parser = token)]
    pub kernel: String,
    /// Release codename or number, such as noble, bookworm or 40
    #[arg(long, value_parser = token)]
    pub release: Option<String>,
    /// Target architecture
    #[arg(long, default_value = std::env::consts::ARCH, value_parser = architecture)]
    pub arch: String,
    /// Exact package version when several builds share a kernel release
    #[arg(long, value_name = "VERSION", conflicts_with = "vmlinux", value_parser = token)]
    pub package_version: Option<String>,
    /// Debug package URL (.deb, .ddeb, .rpm or .apk)
    #[arg(long, value_name = "URL", conflicts_with = "vmlinux", value_parser = package_url)]
    pub package_url: Option<String>,
    /// Local kernel ELF with DWARF, skipping the download
    #[arg(long, value_name = "PATH")]
    pub vmlinux: Option<PathBuf>,
    /// System.map from the same build
    #[arg(long, value_name = "PATH")]
    pub system_map: Option<PathBuf>,
    /// Banner the generated symbols must match exactly
    #[arg(long)]
    pub banner: Option<String>,
    /// dwarf2json to use instead of the bundled one
    #[arg(long, value_name = "PATH")]
    pub dwarf2json: Option<PathBuf>,
    /// Volatility 3 symbols directory
    #[arg(long, value_name = "PATH")]
    pub symbols_dir: Option<PathBuf>,
    /// Directory for temporary downloads and extracted files
    #[arg(long, value_name = "PATH")]
    pub work_dir: Option<PathBuf>,
    /// Regenerate even if already installed
    #[arg(long)]
    pub force: bool,
}

fn token(value: &str) -> std::result::Result<String, String> {
    if value.is_empty()
        || !value.starts_with(|c: char| c.is_ascii_alphanumeric())
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".+-_:~".contains(&b))
    {
        return Err("use a name/version beginning with a letter or digit and containing only letters, digits, . + - _ : ~".into());
    }
    Ok(value.into())
}

fn architecture(value: &str) -> std::result::Result<String, String> {
    Ok(match value {
        "x86_64" | "amd64" => "amd64",
        "aarch64" | "arm64" => "arm64",
        "x86" | "i386" | "i686" => "i386",
        "arm" | "armv7" | "armhf" => "armhf",
        "armel" => "armel",
        "ppc64le" | "ppc64el" => "ppc64el",
        "s390x" => "s390x",
        "riscv64" => "riscv64",
        "loongarch64" => "loongarch64",
        _ => return Err(format!("unsupported architecture: {value}")),
    }
    .into())
}

fn package_url(value: &str) -> std::result::Result<String, String> {
    let url = reqwest::Url::parse(value).map_err(|e| e.to_string())?;
    if !matches!(url.scheme(), "https" | "http") || url.host_str().is_none() {
        return Err("package URL must use HTTP or HTTPS".into());
    }
    package_format(url.path()).map_err(|e| e.to_string())?;
    Ok(value.into())
}

fn package_format(path: &str) -> Result<PackageFormat> {
    if path.ends_with(".deb") || path.ends_with(".ddeb") {
        Ok(PackageFormat::Deb)
    } else if path.ends_with(".rpm") {
        Ok(PackageFormat::Rpm)
    } else if path.ends_with(".apk") {
        Ok(PackageFormat::Apk)
    } else {
        bail!("Package URL must name a .deb, .ddeb, .rpm or .apk file");
    }
}

fn filename(args: &GenerateArgs) -> String {
    let parts = [
        Some(args.distro.to_ascii_lowercase()),
        args.release.clone(),
        Some(args.kernel.clone()),
        Some(args.arch.clone()),
        args.package_version.clone(),
    ];
    let encoded = parts
        .into_iter()
        .flatten()
        .map(|part| {
            // Underscores delimit components; encode them and percent signs to keep
            // distinct specifications from silently reusing the same output path.
            part.bytes()
                .map(|b| {
                    if b.is_ascii_alphanumeric() || b".+-".contains(&b) {
                        (b as char).to_string()
                    } else {
                        format!("%{b:02X}")
                    }
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>();
    format!("{}.json", encoded.join("_"))
}

pub fn cmd_generate(mut args: GenerateArgs) -> Result<()> {
    args.distro = args.distro.to_ascii_lowercase();
    let automatic = args.vmlinux.is_none() && args.package_url.is_none();
    if automatic {
        if !matches!(args.distro.as_str(), "ubuntu" | "debian" | "fedora") {
            return Err(user_error(
                format!("No package lookup for {}", args.distro),
                Some("--package-url <url> or --vmlinux <path>"),
            ));
        }
        if args.release.is_none() && args.distro != "fedora" {
            let example = if args.distro == "ubuntu" {
                "noble"
            } else {
                "bookworm"
            };
            return Err(user_error(
                format!("{} needs a release codename", args.distro),
                Some(&format!("--release {example}")),
            ));
        }
    }
    if let Some(banner) = &args.banner {
        check_banner(banner, &args.kernel, None)?;
    }
    if let Some(map) = &args.system_map {
        ensure_file(map, "System.map")?;
    }
    if let Some(elf) = &args.vmlinux {
        check_elf(elf, &args.arch)?;
    }

    let symbols = match &args.symbols_dir {
        Some(path) => path.clone(),
        None => find_symbols_dir()?,
    };
    let dest = symbols.join("linux");
    fs::create_dir_all(&dest).with_context(|| format!("Cannot create {}", dest.display()))?;
    let destination = dest.join(filename(&args));
    if destination.exists() && !args.force {
        validate_isf(
            BufReader::new(File::open(&destination)?),
            &args.kernel,
            args.banner.as_deref(),
        )
        .context("Existing symbol does not match the request; use --force to regenerate it")?;
        println!("{}  {}", muted("already installed"), destination.display());
        return Ok(());
    }

    let workspace = match &args.work_dir {
        Some(parent) => tempfile::Builder::new()
            .prefix("vol3sm-")
            .tempdir_in(parent),
        None => tempfile::Builder::new().prefix("vol3sm-").tempdir(),
    }
    .context("Cannot create temporary workspace; use --work-dir <existing directory>")?;
    let converter = match &args.dwarf2json {
        Some(path) => path.clone(),
        None => crate::bundled::unpack(workspace.path())?,
    };
    preflight(
        &converter,
        "--help",
        "Use --work-dir on a filesystem that allows executables, or pass --dwarf2json <path>",
    )?;

    let mut system_map = args.system_map.clone();
    let elf = if let Some(elf) = &args.vmlinux {
        elf.clone()
    } else {
        let client = packages::client()?;
        let package = if let Some(url) = &args.package_url {
            Package {
                url: url.clone(),
                version: args.package_version.clone().unwrap_or_default(),
                sha256: None,
                format: package_format(reqwest::Url::parse(url)?.path())?,
            }
        } else {
            if args.distro != "fedora" {
                preflight(
                    Path::new("xz"),
                    "--version",
                    "Install xz-utils (or xz) to read package indexes",
                )?;
            }
            let pb = ui::activity("finding debug package");
            let package = match args.distro.as_str() {
                "fedora" => packages::resolve_fedora(
                    &client,
                    &args.kernel,
                    &args.arch,
                    args.release.as_deref(),
                    args.package_version.as_deref(),
                ),
                _ => packages::resolve(
                    &client,
                    &args.distro,
                    args.release.as_deref().unwrap(),
                    &args.kernel,
                    &args.arch,
                    args.package_version.as_deref(),
                    workspace.path(),
                ),
            };
            pb.finish_and_clear();
            package?
        };
        let extractor = match package.format {
            PackageFormat::Deb => "dpkg-deb",
            _ => "bsdtar",
        };
        preflight(
            Path::new(extractor),
            "--version",
            "Install dpkg-deb for .deb/.ddeb packages, or bsdtar (libarchive-tools) for RPM/APK packages",
        )?;
        println!("{}", package_label(&package.url));
        let archive = workspace.path().join("kernel.package");
        packages::download(&client, &package, &archive)?;
        if matches!(package.format, PackageFormat::Deb) {
            check_deb(&archive, &args, &package.version)?;
        }
        let root = workspace.path().join("extracted");
        fs::create_dir(&root)?;
        let pb = ui::activity("extracting");
        let extracted = match package.format {
            PackageFormat::Deb => Command::new("dpkg-deb")
                .arg("--extract")
                .arg(&archive)
                .arg(&root)
                .output(),
            _ => Command::new("bsdtar")
                .arg("-xf")
                .arg(&archive)
                .arg("-C")
                .arg(&root)
                .output(),
        };
        pb.finish_and_clear();
        check_output(
            extracted.context("Cannot start package extraction")?,
            "Package extraction",
        )?;
        let (elf, map) = find_kernel_files(&root, &args.kernel)?;
        if system_map.is_none() {
            system_map = map;
        }
        elf
    };
    check_elf(&elf, &args.arch)?;
    let pb = ui::activity("running dwarf2json");
    let generated = generate(&args, &converter, &elf, system_map.as_deref(), &destination);
    pb.finish_and_clear();
    generated?;
    println!("{}  {}", success("installed"), destination.display());
    Ok(())
}

/// The package file name, with its directory muted, identifies the exact build.
fn package_label(url: &str) -> String {
    match url.rsplit_once('/') {
        Some((dir, file)) => format!("{}{}", muted(&format!("{dir}/")), file),
        None => url.to_string(),
    }
}

fn preflight(program: &Path, option: &str, hint: &str) -> Result<()> {
    let output = Command::new(program)
        .arg(option)
        .output()
        .with_context(|| format!("Cannot run {}. {hint}", program.display()))?;
    check_output(output, &format!("{} {option}", program.display()))
}

fn check_output(output: std::process::Output, label: &str) -> Result<()> {
    if !output.status.success() {
        bail!(
            "{label} failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

fn ensure_file(path: &Path, label: &str) -> Result<()> {
    if !path.is_file() {
        bail!("{label} is not a file: {}", path.display());
    }
    Ok(())
}

fn check_elf(path: &Path, arch: &str) -> Result<()> {
    let mut header = [0u8; 20];
    let mut file =
        File::open(path).with_context(|| format!("Cannot open kernel ELF {}", path.display()))?;
    if file.read_exact(&mut header).is_err() || &header[..4] != b"\x7fELF" {
        return Err(user_error(
            format!("{} is not a kernel ELF", path.display()),
            Some("Use an unstripped vmlinux with DWARF, not vmlinuz or headers"),
        ));
    }
    let machine = match header[5] {
        1 => u16::from_le_bytes([header[18], header[19]]),
        2 => u16::from_be_bytes([header[18], header[19]]),
        _ => bail!("Invalid ELF byte order"),
    };
    let expected = match arch {
        "amd64" => 62,
        "i386" => 3,
        "arm64" => 183,
        "armhf" | "armel" => 40,
        "ppc64el" => 21,
        "s390x" => 22,
        "riscv64" => 243,
        "loongarch64" => 258,
        _ => bail!("Unsupported architecture {arch}"),
    };
    if machine != expected {
        bail!(
            "Kernel ELF architecture does not match --arch {arch} (ELF machine {machine}, expected {expected})"
        );
    }
    Ok(())
}

fn check_deb(archive: &Path, args: &GenerateArgs, version: &str) -> Result<()> {
    let output = Command::new("dpkg-deb")
        .arg("--field")
        .arg(archive)
        .output()?;
    if !output.status.success() {
        return check_output(output, "Reading package metadata");
    }
    let metadata = String::from_utf8(output.stdout)?;
    let field = |key: &str| {
        metadata
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{key}: ")))
    };
    if field("Architecture") != Some(args.arch.as_str()) {
        bail!(
            "Debug package architecture does not match --arch {}",
            args.arch
        );
    }
    if !version.is_empty() && field("Version") != Some(version) {
        bail!("Debug package does not match requested package version {version}");
    }
    if matches!(args.distro.as_str(), "ubuntu" | "debian")
        && !packages::package_names(&args.distro, &args.kernel)
            .iter()
            .any(|name| field("Package") == Some(name.as_str()))
    {
        bail!(
            "Package does not contain the requested {} kernel {} debug symbols",
            args.distro,
            args.kernel
        );
    }
    Ok(())
}

fn find_kernel_files(root: &Path, kernel: &str) -> Result<(PathBuf, Option<PathBuf>)> {
    fn walk(dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            // Do not follow absolute package symlinks back into the host.
            if kind.is_dir() {
                walk(&entry.path(), files)?;
            } else if kind.is_file() {
                files.push(entry.path());
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    walk(root, &mut files)?;
    let mut elves = Vec::new();
    let mut maps = Vec::new();
    for path in files {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        let in_kernel_dir = path.components().any(|c| c.as_os_str() == kernel);
        if name == format!("vmlinux-{kernel}") || (name == "vmlinux" && in_kernel_dir) {
            elves.push(path);
        } else if name == format!("System.map-{kernel}") || (name == "System.map" && in_kernel_dir)
        {
            maps.push(path);
        }
    }
    elves.sort();
    maps.sort();
    match elves.len() {
        0 => bail!(
            "Package has no vmlinux for kernel {kernel}. A matching kernel debug package with DWARF is required; use --vmlinux to supply the debug ELF directly"
        ),
        1 => Ok((elves.remove(0), maps.into_iter().next())),
        _ => bail!(
            "Package has several vmlinux files for {kernel}; use --vmlinux to select the exact debug ELF"
        ),
    }
}

fn generate(
    args: &GenerateArgs,
    converter: &Path,
    elf: &Path,
    map: Option<&Path>,
    destination: &Path,
) -> Result<()> {
    let mut temporary =
        NamedTempFile::new_in(destination.parent().context("Output has no parent")?)?;
    let mut command = Command::new(converter);
    command.arg("linux").arg("--elf").arg(elf);
    if let Some(map) = map {
        command.arg("--system-map").arg(map);
    }
    // Never buffer the potentially multi-gigabyte JSON output in memory.
    let output = command
        .stdout(Stdio::from(temporary.reopen()?))
        .output()
        .context("Cannot start dwarf2json")?;
    check_output(output, "dwarf2json")?;
    validate_isf(
        BufReader::new(temporary.reopen()?),
        &args.kernel,
        args.banner.as_deref(),
    )?;
    temporary.as_file_mut().sync_all()?;
    // Temporary files are created 0600. Installed symbols must be readable like
    // downloaded ones, including by a Volatility run under another account.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(temporary.path(), fs::Permissions::from_mode(0o644))?;
    }
    if args.force {
        temporary.persist(destination).map_err(|e| e.error)?;
    } else {
        temporary
            .persist_noclobber(destination)
            .map_err(|e| e.error)
            .with_context(|| {
                format!(
                    "Cannot install {}; another process may have installed it",
                    destination.display()
                )
            })?;
    }
    Ok(())
}

#[derive(Deserialize)]
struct Isf {
    metadata: Metadata,
    symbols: Symbols,
    user_types: NonEmptyObject,
}
#[derive(Deserialize)]
struct Metadata {
    format: String,
}
#[derive(Deserialize)]
struct Symbols {
    linux_banner: BannerSymbol,
}
#[derive(Deserialize)]
struct BannerSymbol {
    constant_data: String,
}

struct NonEmptyObject;
impl<'de> Deserialize<'de> for NonEmptyObject {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        struct ObjectVisitor;
        impl<'de> Visitor<'de> for ObjectVisitor {
            type Value = NonEmptyObject;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a nonempty type map")
            }
            fn visit_map<M: MapAccess<'de>>(
                self,
                mut map: M,
            ) -> std::result::Result<Self::Value, M::Error> {
                let mut count = 0;
                while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {
                    count += 1;
                }
                if count == 0 {
                    return Err(serde::de::Error::custom(
                        "No DWARF types; use an unstripped kernel debug ELF",
                    ));
                }
                Ok(NonEmptyObject)
            }
        }
        deserializer.deserialize_map(ObjectVisitor)
    }
}

fn validate_isf(reader: impl Read, kernel: &str, expected_banner: Option<&str>) -> Result<()> {
    let isf: Isf =
        serde_json::from_reader(reader).context("dwarf2json did not produce a valid Linux ISF")?;
    let _types = isf.user_types;
    if isf.metadata.format.is_empty() {
        bail!("Generated ISF lacks a format version");
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(isf.symbols.linux_banner.constant_data)
        .context("Generated ISF has an invalid linux_banner")?;
    let banner = std::str::from_utf8(&bytes).context("Generated kernel banner is not UTF-8")?;
    check_banner(banner, kernel, expected_banner)
}

fn check_banner(banner: &str, kernel: &str, expected: Option<&str>) -> Result<()> {
    let banner = banner.trim_end_matches('\0').trim_end();
    if !banner.starts_with(&format!("Linux version {kernel} ")) {
        bail!("Kernel banner does not match requested kernel {kernel}: {banner}");
    }
    if let Some(expected) = expected
        && banner != expected.trim_end_matches('\0').trim_end()
    {
        bail!(
            "Generated kernel banner does not exactly match --banner; select the matching debug package build"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn isf(banner: &str, types: &str) -> String {
        let encoded = base64::engine::general_purpose::STANDARD.encode(banner);
        format!(
            r#"{{"metadata":{{"format":"6.2.0"}},"symbols":{{"linux_banner":{{"constant_data":"{encoded}"}}}},"user_types":{types}}}"#
        )
    }

    #[test]
    fn validates_isf_and_rejects_wrong_build_empty_types_or_truncation() {
        let banner = "Linux version 6.8.0-31-generic (buildd) #31 Ubuntu\n\0";
        let good = isf(banner, r#"{"task_struct":{"size":123}}"#);
        validate_isf(good.as_bytes(), "6.8.0-31-generic", Some(banner)).unwrap();
        assert!(validate_isf(good.as_bytes(), "6.8.0-32-generic", None).is_err());
        assert!(
            validate_isf(
                good.as_bytes(),
                "6.8.0-31-generic",
                Some("Linux version 6.8.0-31-generic other build")
            )
            .is_err()
        );
        assert!(validate_isf(isf(banner, "{}").as_bytes(), "6.8.0-31-generic", None).is_err());
        assert!(
            validate_isf(&good.as_bytes()[..good.len() - 1], "6.8.0-31-generic", None).is_err()
        );
    }

    #[test]
    fn finds_only_exact_kernel_files_and_new_ubuntu_layout() {
        let dir = tempfile::tempdir().unwrap();
        let modules = dir
            .path()
            .join("usr/lib/debug/lib/modules/6.8.0-31-generic");
        fs::create_dir_all(&modules).unwrap();
        fs::write(modules.join("vmlinux"), b"elf").unwrap();
        fs::write(modules.join("System.map"), b"map").unwrap();
        let (elf, map) = find_kernel_files(dir.path(), "6.8.0-31-generic").unwrap();
        assert_eq!(elf, modules.join("vmlinux"));
        assert_eq!(map.unwrap(), modules.join("System.map"));
        assert!(find_kernel_files(dir.path(), "6.8.0-32-generic").is_err());
    }

    #[test]
    fn rejects_invalid_paths_urls_and_architectures() {
        for bad in ["../noble", "--help", "a/b", "$(id)", "", "."] {
            assert!(token(bad).is_err());
        }
        assert_eq!(architecture("aarch64").unwrap(), "arm64");
        assert!(architecture("mystery").is_err());
        assert!(package_url("file:///tmp/kernel.deb").is_err());
        assert!(package_url("https://example.org/headers.tar.gz").is_err());
        assert!(package_url("https://example.org/kernel.rpm?download=1").is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn generation_installs_atomically_and_preserves_existing_symbols_on_failure() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let elf = root.path().join("vmlinux with spaces");
        let mut header = [0u8; 20];
        header[..4].copy_from_slice(b"\x7fELF");
        header[5] = 1;
        header[18] = 62;
        fs::write(&elf, header).unwrap();
        let tool = root.path().join("dwarf2json with spaces");
        fs::write(&tool, "#!/bin/sh\nif [ \"$1\" = --help ]; then exit 0; fi\n[ \"$1\" = linux ] && [ \"$2\" = --elf ] || exit 2\ncat \"$0.json\"\n").unwrap();
        fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();
        let data = root.path().join("dwarf2json with spaces.json");
        let good = isf(
            "Linux version 6.8.0-31-generic fixture\n\0",
            r#"{"task_struct":{}}"#,
        );
        fs::write(&data, &good).unwrap();
        let make_args = || GenerateArgs {
            distro: "ubuntu".into(),
            kernel: "6.8.0-31-generic".into(),
            release: Some("noble".into()),
            arch: "amd64".into(),
            package_version: None,
            package_url: None,
            vmlinux: Some(elf.clone()),
            system_map: None,
            banner: None,
            dwarf2json: Some(tool.clone()),
            symbols_dir: Some(root.path().join("symbols")),
            work_dir: Some(root.path().into()),
            force: false,
        };
        let destination = root
            .path()
            .join("symbols/linux")
            .join(filename(&make_args()));
        cmd_generate(make_args()).unwrap();
        assert_eq!(fs::read_to_string(&destination).unwrap(), good);
        assert_eq!(
            fs::metadata(&destination).unwrap().permissions().mode() & 0o777,
            0o644
        );
        // Reuse works even without a converter installed.
        let mut repeat = make_args();
        repeat.dwarf2json = Some(root.path().join("missing converter"));
        cmd_generate(repeat).unwrap();

        let mut replace = make_args();
        replace.force = true;
        fs::write(&data, "{truncated").unwrap();
        assert!(cmd_generate(replace).is_err());
        assert_eq!(fs::read_to_string(&destination).unwrap(), good);
        assert_eq!(
            fs::read_dir(destination.parent().unwrap()).unwrap().count(),
            1
        );
        assert!(!fs::read_dir(root.path()).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("vol3sm-")
        }));

        // A converter can exit successfully while emitting the wrong kernel.
        fs::remove_file(&destination).unwrap();
        fs::write(
            &data,
            isf(
                "Linux version 6.8.0-32-generic wrong\n\0",
                r#"{"task_struct":{}}"#,
            ),
        )
        .unwrap();
        assert!(cmd_generate(make_args()).is_err());
        assert!(!destination.exists());
        assert_eq!(
            fs::read_dir(destination.parent().unwrap()).unwrap().count(),
            0
        );
    }

    #[test]
    fn rejects_wrong_elf_architecture() {
        let file = NamedTempFile::new().unwrap();
        let mut header = [0u8; 20];
        header[..4].copy_from_slice(b"\x7fELF");
        header[5] = 1;
        header[18] = 183;
        fs::write(file.path(), header).unwrap();
        check_elf(file.path(), "arm64").unwrap();
        assert!(check_elf(file.path(), "amd64").is_err());
    }
}
