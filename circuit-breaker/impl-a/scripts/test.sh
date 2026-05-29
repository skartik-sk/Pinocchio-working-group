#!/bin/bash

# Build and test impl-a
set -e

echo "Building impl-a..."
cd impl-a
cargo build-sbf --verifiable-build

echo "Running impl-a tests..."
anchor test

echo "impl-a tests passed!"
