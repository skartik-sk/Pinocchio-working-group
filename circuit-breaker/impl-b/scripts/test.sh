#!/bin/bash

# Build and test impl-b
set -e

echo "Building impl-b..."
cd impl-b
cargo build-sbf --verifiable-build

echo "Running impl-b tests..."
anchor test

echo "impl-b tests passed!"
