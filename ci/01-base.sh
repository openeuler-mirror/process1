#!/bin/bash
set -e

#开始检查
cargo fmt --all -- --check -v

#cargo clippy --all-targets --all-features --tests --benches -- -D warnings
cargo clippy --all-targets --all-features --tests --benches -- -v
cargo clean

#cargo check
#cargo clean

#cargo rustc -- -D warnings
#bin=$(sed -n '/[[bin]]/ {n;p}' Cargo.toml | sed 's/\"//g' | sed 's/name = //g')
#for bin_name in $bin
#do
#echo $bin_name
#cargo rustc --bin $bin_name -- -D warnings -v
#done

cargo build --release --all -v
RUST_BACKTRACE=1 cargo test --all --release -v -- --nocapture --test-threads=1
#RUST_BACKTRACE=1 cargo test --all  --release -v -- --nocapture
