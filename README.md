# vol3-symbol-manager (`vol3sm`)

A fast, CLI-based symbol manager for [Volatility 3](https://github.com/volatilityfoundation/volatility3). 

`vol3sm` allows you to effortlessly search for and install Intermediate Symbol Format (ISF) files directly from public community repositories, saving you the hassle of manually downloading and placing JSON files in the correct directories.

## Features

- **Fast, Fuzzy Searching**: Quickly find symbols by kernel banner, distro name, kernel version, architecture, or filename snippet.
- **Automated Installation**: Downloads and correctly places `.json` or `.json.xz` symbol files into your local Volatility 3 symbols directory.
- **Multiple Sources**: Aggregates symbols from popular community-maintained repositories.
- **Local Management**: List your locally installed symbols and instantly locate your Volatility 3 symbols directory.
- **On-demand Generation**: Resolve an exact Ubuntu, Debian, or Fedora kernel debug package, download and extract it, run the bundled `dwarf2json`, and install the generated Linux ISF.

## Installation

Building from source requires [Rust and Cargo](https://rustup.rs/) and [Go 1.18+](https://go.dev/dl/) on `PATH`. Go compiles the vendored `dwarf2json` into the `vol3sm` binary; it is not required at runtime. The converter and its Go dependency are pinned and vendored, so this build step needs no network access. Then, clone the repository and install it:

```bash
git clone https://github.com/yourusername/vol3-symbol-manager.git
cd vol3-symbol-manager
cargo install --path .
```

This will place the `vol3sm` binary in your `~/.cargo/bin` directory.

## Usage

You can view the help menu at any time by running:
```bash
vol3sm --help
```

### 1. Search for Symbols
Search using snippets of a banner, kernel version, or OS name. The search utilizes a fuzzy-matching and ranking algorithm to bring the most relevant results to the top.

```bash
vol3sm search "ubuntu 6.2.0 aws"
```
```
8 of 54 matches for ubuntu 6.2.0 aws

   abyss:Ubuntu/amd64/6.2.0/
0  ├── Ubuntu_6.2.0-1004-aws_6.2.0-1004.4_amd64.json.xz
1  ├── Ubuntu_6.2.0-1005-aws_6.2.0-1005.5_amd64.json.xz
...

install  vol3sm install <id>
more     --limit 54
```

### 2. Install a Symbol
You can install a symbol easily using the id printed by your last search, or by providing the exact kernel banner.

**By ID (from previous search):**
```bash
vol3sm install 0
```

**By Source Path:**
```bash
vol3sm install "abyss:Ubuntu/amd64/6.2.0/Ubuntu_6.2.0-1007-aws_6.2.0-1007.7_amd64.json.xz"
```

**By Exact Banner:**
*(Tip: get the banner from your memory dump by running `vol.py -r pretty -f <dump> banners`)*
```bash
vol3sm install --banner "Linux version 6.2.0-1007-aws (buildd@lcy02-amd64-077) ..."
```

### 3. Generate Symbols on Demand

When a community symbol is unavailable, generate one from the target kernel's
debug package. Use the **target's full `uname -r`**, including its flavor and, for
Fedora, architecture. The target can differ from the machine running `vol3sm`.

```bash
# Ubuntu: release codename and exact kernel release
vol3sm generate --distro ubuntu --release noble --kernel 6.8.0-31-generic --arch amd64

# Debian: release codename and exact kernel release
vol3sm generate --distro debian --release bookworm --kernel 6.1.0-47-amd64 --arch amd64

# Fedora: the full kernel release identifies the build in Fedora's Koji archive
vol3sm generate --distro fedora --kernel 6.8.5-301.fc40.x86_64 --arch x86_64
```

Automatic lookup uses Ubuntu's `ddebs` indexes (release, updates, proposed),
Debian's regular kernel package indexes (release, updates, backports, security),
or Fedora's Koji archive. The requested build must still be available there;
`vol3sm` never substitutes a nearby kernel. If several package revisions match
an Ubuntu/Debian kernel ABI, specify `--package-version` to select the exact
build. Ubuntu's unsigned debug payload is preferred over dependency-only signed
package aliases. Ubuntu/Debian downloads are checked against the index SHA-256.

`dwarf2json` 0.9.0 is **bundled inside the `vol3sm` executable**. Generation
automatically extracts it into the private workspace and cleans it up afterward;
no separate converter installation or runtime Go installation is needed. Use
`--dwarf2json /path/to/dwarf2json` to override it. Run `vol3sm licenses` for the
bundled software's notices and licenses.

Package lookup/extraction requires these programs on `PATH`:

- `xz` and `dpkg-deb` for automatic Ubuntu/Debian lookup and extraction.
- `bsdtar` (usually the `libarchive-tools` or `libarchive` package) for RPM extraction.

The debug package is extracted in a temporary workspace without installing it
or running package scripts. Kernel conversion can require at least **8 GB RAM**
and several GB of temporary disk space. Use `--work-dir /large/existing/directory`
to choose where downloads and extracted files are stored; these are cleaned up
on success or failure. That filesystem must allow executing the bundled converter.

The resulting `.json` is installed under `<symbols directory>/linux/`, using
the existing symbols directory detection or `VOL3_SYMBOLS_DIR`. You can also
pass `--symbols-dir /path/to/volatility3/symbols` to select/create a symbols root.
Generated symbols appear in `vol3sm local` like downloaded symbols. Repeated
requests reuse the installed symbol; `--force` replaces it only after the new
output passes validation. The ELF architecture, kernel banner, JSON structure,
and presence of DWARF types are checked before installation.

For an exact build check, supply the full banner from the memory dump:

```bash
vol3sm generate --distro ubuntu --release noble --kernel 6.8.0-31-generic \
  --package-version 6.8.0-31.31 --banner "Linux version 6.8.0-31-generic ..."
```

Replace the example banner with the **complete actual banner**, not an
abbreviated string. A matching `System.map` from the debug package is used when
available; `--system-map /path/to/System.map` supplies one explicitly. It must
come from the same target build.

For archived/custom packages, or other distributions such as Alpine, bypass
automatic lookup with a direct debug-package URL or an unstripped local kernel
ELF containing DWARF:

```bash
vol3sm generate --distro ubuntu --kernel 6.8.0-31-generic --arch amd64 \
  --package-url 'https://example.org/linux-image-unsigned-6.8.0-31-generic-dbgsym_6.8.0-31.31_amd64.ddeb'

vol3sm generate --distro alpine --kernel 6.12.117-0-lts --arch x86_64 \
  --vmlinux /path/to/debug/vmlinux --system-map /path/to/System.map
```

Direct URLs support `.deb`/`.ddeb`, `.rpm`, and debug `.apk` archives supported
by the installed `bsdtar`. Boot kernels (`vmlinuz`) and header packages do not
contain the DWARF information required by `dwarf2json`. Alpine has no automatic
package lookup; supply the matching debug ELF or custom debug package.

### 4. List Local Symbols
View all the symbol files currently installed in your Volatility 3 symbols directory:

```bash
vol3sm local
```

### 5. Locate Symbols Directory
Print the absolute path to where Volatility 3 is configured to look for symbols on your system:

```bash
vol3sm symbols-dir
```

## Supported Symbol Indexes

Currently, `vol3sm` aggregates and fetches symbols from the following community repositories:
- [Abyss-W4tcher/volatility3-symbols](https://github.com/Abyss-W4tcher/volatility3-symbols)
- [leludo84/vol3-linux-profiles](https://github.com/leludo84/vol3-linux-profiles)
- [p0dalirius/volatility3-symbols](https://github.com/p0dalirius/volatility3-symbols)
