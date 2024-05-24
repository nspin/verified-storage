#!/usr/bin/env bash

verus_dir=verus

url="https://github.com/nspin/verus"
branch="pr/cargo-integration"
common_args="--git $url --branch $branch --root $out_dir"

cargo install $common_args cargo-verus verus-driver

export PATH=$(realpath $verus_dir)/bin:$PATH

cargo verus
