#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")"

BASE_URL="https://data.binance.vision/data/futures/um/daily/aggTrades/BTCUSDT"

for DATE in 2026-04-21 2026-04-22 2026-04-23 2026-04-24 2026-04-25 2026-04-26 2026-04-27; do
    FILE="BTCUSDT-aggTrades-${DATE}.csv"
    ZIP="BTCUSDT-aggTrades-${DATE}.zip"

    if [ -f "$FILE" ] && [ -s "$FILE" ]; then
        echo "SKIP: $FILE already exists"
        continue
    fi

    echo "Downloading $ZIP..."
    curl -fSL "${BASE_URL}/${ZIP}" -o "$ZIP"
    unzip -o "$ZIP"
    rm -f "$ZIP"

    if [ ! -s "$FILE" ]; then
        echo "ERROR: $FILE is empty after extraction"
        exit 1
    fi

    LINE_COUNT=$(wc -l < "$FILE")
    if [ "$LINE_COUNT" -lt 10 ]; then
        echo "ERROR: $FILE has only $LINE_COUNT lines"
        exit 1
    fi

    SAMPLE=$(head -1 "$FILE" | awk -F, '{print NF}')
    if [ "$SAMPLE" -lt 7 ]; then
        echo "ERROR: $FILE has $SAMPLE columns, expected ≥7"
        exit 1
    fi

    echo "OK: $FILE ($LINE_COUNT lines)"
done

echo ""
echo "All files validated:"
ls -lh BTCUSDT-aggTrades-2026-04-2*.csv
