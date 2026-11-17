//! The converter is built for the Rust target and embedded in the single CLI.
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

const DWARF2JSON: &[u8] = include_bytes!(env!("VOL3SM_BUNDLED_DWARF2JSON"));

pub fn unpack(workspace: &Path) -> Result<PathBuf> {
    let path = workspace.join(if cfg!(windows) {
        "dwarf2json.exe"
    } else {
        "dwarf2json"
    });
    std::fs::write(&path, DWARF2JSON).context("Cannot unpack bundled dwarf2json")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(path)
}

pub fn print_licenses() {
    println!(
        "Bundled dwarf2json 0.9.0\nSource: https://github.com/volatilityfoundation/dwarf2json\nRevision: 9f14607e0d339d463ea725fbd5c08aa7b7d40f75\n\n{}\n\npflag v1.0.5\nSource: https://github.com/spf13/pflag\n\n{}",
        include_str!("../vendor/dwarf2json/LICENSE.txt"),
        include_str!("../vendor/dwarf2json/vendor/github.com/spf13/pflag/LICENSE")
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn embedded_converter_runs_without_path_lookup() {
        let dir = tempfile::tempdir().unwrap();
        let executable = unpack(dir.path()).unwrap();
        let output = std::process::Command::new(executable)
            .arg("--version")
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("0.9.0"));
    }
}
