# xtool

A simple command line utility for encoding, decoding and hashing files.

Instead of having separate binaries for `base32`, `base64`, `base85`, `hex`, `binary`, `sha256sum` and `sha512sum`, `xtool` provides all of them in a single executable.

You can either use it as:

```bash
xtool <command>
```

or create symlinks and use it like regular coreutils commands:

```bash
base64 file.txt
sha256sum file.txt
hex -d dump.txt
```

---

## Features

* Base32 encode/decode
* Base64 encode/decode
* Base85 encode/decode
* Hex encode/decode
* Binary encode/decode
* SHA-256 checksums
* SHA-512 checksums
* Streaming processing for large files
* Supports stdin and file input
* Coreutils-style symlink dispatch
* Memory-mapped file reads where possible

---

## Installation

Build the project:

```bash
cargo build --release
```

The binary will be available at:

```bash
target/release/xtool
```

Create symlinks if you want to call the tool directly as `base64`, `sha256sum`, etc.

```bash
ln -s /path/to/xtool base32
ln -s /path/to/xtool base64
ln -s /path/to/xtool base85
ln -s /path/to/xtool hex
ln -s /path/to/xtool binary
ln -s /path/to/xtool sha256sum
ln -s /path/to/xtool sha512sum
```

Place the symlinks anywhere in your `PATH`.

The binary automatically detects which mode to run based on the name it was started with.

---

## Usage

### Using xtool subcommands

```bash
xtool base64 file.txt
xtool base64 -d encoded.txt

xtool base32 file.txt
xtool base32 -d encoded.txt

xtool hex file.bin
xtool hex -d dump.txt

xtool binary file.bin
xtool binary -d bits.txt

xtool sha256sum file.txt
xtool sha512sum file.txt
```

### Using symlinks

```bash
base64 file.txt
base64 -d encoded.txt

base32 file.txt
base32 -d encoded.txt

hex file.bin
hex -d dump.txt

binary file.bin
binary -d bits.txt

sha256sum file.txt
sha512sum file.txt
```

---

## Reading From Stdin

All commands support stdin.

Encode:

```bash
echo "hello" | xtool base64
```

Decode:

```bash
echo "aGVsbG8K" | xtool base64 -d
```

Hash:

```bash
cat file.txt | xtool sha256sum
```

You can also pass `-` explicitly:

```bash
cat file.txt | base64 -
```

---

## Supported Commands

| Command     | Description                                            |
| ----------- | ------------------------------------------------------ |
| `base32`    | Encode or decode Base32                                |
| `base64`    | Encode or decode Base64                                |
| `base85`    | Encode or decode Base85                                |
| `hex`       | Encode bytes to hex and decode hex back to bytes       |
| `binary`    | Encode bytes to binary and decode binary back to bytes |
| `sha256sum` | Print SHA-256 checksum                                 |
| `sha512sum` | Print SHA-512 checksum                                 |

For encoding commands:

```bash
-d
--decode
```

switches from encoding mode to decoding mode.

---

## Examples

### Encode a file as Base64

```bash
xtool base64 image.png > image.b64
```

### Decode it back

```bash
xtool base64 -d image.b64 > image_restored.png
```

### Generate SHA-256 checksum

```bash
xtool sha256sum file.txt
```

Output:

```text
d2a84f4b8b650937ec8f73cd8be2c74add5a911ba64df27458ed8229da804a26  file.txt
```

### Hash multiple files

```bash
xtool sha256sum file1.txt file2.txt file3.txt
```

### Convert text to binary

```bash
echo -n "hi" | xtool binary
```

Output:

```text
0110100001101001
```

### Convert hex back to text

```bash
echo "48656c6c6f" | xtool hex -d
```

Output:

```text
Hello
```

---

## Notes

* Base32, Base64, Hex, Binary, SHA-256 and SHA-512 operate in streaming mode and process data in chunks.
* Large files can be processed without loading the entire file into memory.
* Base85 currently buffers the whole input because the underlying crate does not provide a streaming API.
* Encode/decode commands accept a single input file or stdin.
* SHA-256 and SHA-512 support multiple input files.

---

## Dependencies
See [Cargo.toml](./Cargo.toml)
---

## License

APACHE 2
