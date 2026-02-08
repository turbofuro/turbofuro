#!/bin/bash

# Check if jq is installed
if ! command -v jq &> /dev/null; then
    echo "Error: jq is not installed. Please install it to continue."
    exit 1
fi

# Iterate over all .json files in the current directory
for file in *.json; do
    # Check if any json files exist to avoid errors in empty directories
    [ -e "$file" ] || continue

    echo "Processing $file..."

    # Sort keys and save to a temporary file
    # Use the --sort-keys (or -S) flag
    if jq --sort-keys '.' "$file" > "$file.tmp"; then
        mv "$file.tmp" "$file"
        echo "Successfully sorted $file"
    else
        echo "Failed to process $file. Check if the JSON is valid."
        rm -f "$file.tmp"
    fi
done

echo "Done!"