#!/usr/bin/env bash

verus_dir=verus

cargo install \
    --git 'https://github.com/nspin/verus' \
    --branch pr/cargo-integration \
    --root $verus_dir \
    cargo-verus verus-driver

export PATH=$(cd $verus_dir && pwd)/bin:$PATH

cargo verus
