#!/bin/bash
# Script to update test files to use new database-aware API

# Define the database name to use in tests
DB="_system"

# Function to update a test file
update_test_file() {
    local file=$1
    echo "Updating $file..."
    
    # Backup original
    cp "$file" "$file.bak"
    
    # Update collection routes
    sed -i '' "s|\"/_api/collection\"|\"/_api/database/$DB/collection\"|g" "$file"
    sed -i '' "s|/_api/collection/|/_api/database/$DB/collection/|g" "$file"
    
    # Update document routes
    sed -i '' "s|\"/_api/document/|\"/_api/database/$DB/document/|g" "$file"
    sed -i '' "s|/_api/document/|/_api/database/$DB/document/|g" "$file"
    
    # Update cursor/query routes
    sed -i '' "s|\"/_api/cursor\"|\"/_api/database/$DB/cursor\"|g" "$file"
    sed -i '' "s|\"/_api/explain\"|\"/_api/database/$DB/explain\"|g" "$file"
    
    # Update index routes
    sed -i '' "s|\"/_api/index/|\"/_api/database/$DB/index/|g" "$file"
    sed -i '' "s|/_api/index/|/_api/database/$DB/index/|g" "$file"
    
    # Update geo routes
    sed -i '' "s|\"/_api/geo/|\"/_api/database/$DB/geo/|g" "$file"
    sed -i '' "s|/_api/geo/|/_api/database/$DB/geo/|g" "$file"
}

# Update all test files
for file in tests/*.rs; do
    if [ -f "$file" ]; then
        update_test_file "$file"
    fi
done

echo "All test files updated!"
echo "Backups saved with .bak extension"
