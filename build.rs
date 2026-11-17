use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=vendor/dwarf2json");
    println!("cargo:rerun-if-env-changed=VOL3SM_GO");
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo sets OUT_DIR"));
    let target_os = env::var("CARGO_CFG_TARGET_OS").expect("Cargo sets target OS");
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").expect("Cargo sets target architecture");
    let go_os = match target_os.as_str() {
        "macos" => "darwin",
        os => os,
    };
    let go_arch = match target_arch.as_str() {
        "x86_64" => "amd64",
        "x86" => "386",
        "aarch64" => "arm64",
        "powerpc64" if env::var("CARGO_CFG_TARGET_ENDIAN").as_deref() == Ok("little") => "ppc64le",
        "powerpc64" => "ppc64",
        arch => arch,
    };
    let executable = out.join(if target_os == "windows" {
        "dwarf2json.exe"
    } else {
        "dwarf2json"
    });
    let mut build = Command::new(env::var_os("VOL3SM_GO").unwrap_or_else(|| "go".into()));
    build
        .current_dir("vendor/dwarf2json")
        .args([
            "build",
            "-mod=vendor",
            "-trimpath",
            "-buildvcs=false",
            "-ldflags=-s -w",
            "-o",
        ])
        .arg(&executable)
        .arg(".")
        .env("CGO_ENABLED", "0")
        .env("GOOS", go_os)
        .env("GOARCH", go_arch)
        .env("GOPROXY", "off")
        .env("GOSUMDB", "off")
        .env("GOTOOLCHAIN", "local")
        .env("GOCACHE", out.join("go-cache"))
        .env("GOMODCACHE", out.join("go-mod"));
    if go_arch == "arm" {
        build.env("GOARM", "7");
    }
    let result = build.output().expect("Go 1.18+ is required to build the bundled dwarf2json; install Go or set VOL3SM_GO to its executable");
    assert!(
        result.status.success(),
        "Building bundled dwarf2json failed:\n{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    println!(
        "cargo:rustc-env=VOL3SM_BUNDLED_DWARF2JSON={}",
        executable.display()
    );
}
