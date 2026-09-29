set -eu
test "$(node --version)" = v22.19.0
test "$(go version)" = 'go version go1.25.14 linux/amd64'
rustc --version | grep '^rustc 1.95.0 '
rustup target list --installed | grep -x x86_64-unknown-linux-musl
for tool in npm cargo cc c++ make musl-gcc perl pkg-config python3 git readelf; do command -v "$tool"; done
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT
printf 'int main(void) { return 0; }\n' > "$work/probe.c"
musl-gcc -static "$work/probe.c" -o "$work/probe"
"$work/probe"
printf 'fn main() {}\n' > "$work/probe.rs"
rustc --target x86_64-unknown-linux-musl "$work/probe.rs" -o "$work/rust-probe"
"$work/rust-probe"
