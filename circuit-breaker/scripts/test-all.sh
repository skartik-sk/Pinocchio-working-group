#!/bin/bash

# Test both implementations
set -e

echo "Testing impl-a (Escrow + Circuit Breaker)..."
cd impl-a
bash scripts/test.sh

echo ""
echo "Testing impl-b (Stable Swap + Circuit Breaker)..."
cd ../impl-b
bash scripts/test.sh

echo ""
echo "All tests passed! Both implementations working correctly."
