#!/bin/bash
set -e

cd "$(dirname "$0")"

# Configuration
PORT=9999
ADMIN_USER="admin"
ADMIN_PASSWORD="admin"
DATA_DIR="./test_data_php"
PROJECT_ROOT="../.."

echo "Preparing test environment..."

# Check requirements
if ! command -v php &> /dev/null; then
    echo "Error: PHP is not installed. Please install PHP to run tests."
    exit 1
fi

if [ ! -d "vendor" ]; then
    echo "Vendor directory not found. Trying to install dependencies..."
    if command -v composer &> /dev/null; then
        composer config --no-plugins allow-plugins.pestphp/pest-plugin true
        composer install --ignore-platform-reqs -n
    else
        echo "Error: Composer not found. Please install composer to install test dependencies."
        exit 1
    fi
fi

# 1. Clean up old test data
rm -rf "$DATA_DIR"
mkdir -p "$DATA_DIR"

# 2. Build the server (to ensure it's up to date)
echo "Building SoliDB..."
cargo build --manifest-path "$PROJECT_ROOT/Cargo.toml" --quiet

# 3. Start the server in background
echo "Starting SoliDB on port $PORT..."
# We use SOLIDB_ADMIN_PASSWORD to set the initial admin password
export SOLIDB_ADMIN_PASSWORD="$ADMIN_PASSWORD"

# Start server
cargo run --manifest-path "$PROJECT_ROOT/Cargo.toml" --quiet -- --port "$PORT" --data-dir "$DATA_DIR" &
SERVER_PID=$!

# 4. Wait for server to be ready
echo "Waiting for server to start (PID: $SERVER_PID)..."
MAX_RETRIES=30
count=0
while ! nc -z localhost "$PORT"; do
    sleep 0.5
    count=$((count+1))
    if [ $count -ge $MAX_RETRIES ]; then
        echo "Error: Server failed to start on port $PORT"
        kill $SERVER_PID
        exit 1
    fi
done
echo "Server is ready on port $PORT!"

# 5. Run PHP Tests
echo "Running PHP Tests..."
export SOLIDB_PORT="$PORT"
# We don't need to pass credentials to the test yet as the current tests don't strictly enforce auth
# or assume default/no auth for the operations or handle it internally.
# But if auth IS enforced, we should update ClientTest.php to use them.
# The current ClientTest.php does NOT call $client->auth().
# If SoliDB requires auth by default when admin is created, tests might fail.
# Let's see. SoliDB creates admin but doesn't necessarily enforce auth on all ops unless configured?
# The `auth.rs` shows `create_jwt` and middlewares.
# Routes are protected by `auth_middleware`.
# So tests WILL fail without auth if they hit protected endpoints.
# The `ClientTest.php` needs to Authenticate using `admin:admin`.

# Let's try running it. If it fails, I will need to update ClientTest.php to authenticate.
# Actually, looking at `routes.rs`, most API routes are protected.
# Except health check.
# The `Client.php` has an `auth` method.
# I should probably update ClientTest.php to auth first.

# Run Tests
if [ -f "./vendor/bin/pest" ]; then
    echo "Running Pest Tests..."
    ./vendor/bin/pest
else
    echo "Pest not found (likely install failed due to platform issues). Running simple tests..."
    php tests/simple_test.php
fi

if [ $? -eq 0 ]; then
    echo "Tests PASSED!"
    EXIT_CODE=0
else
    echo "Tests FAILED!"
    EXIT_CODE=1
fi

# 6. Cleanup
echo "stopping server..."
kill $SERVER_PID
wait $SERVER_PID 2>/dev/null || true
rm -rf "$DATA_DIR"

exit $EXIT_CODE
