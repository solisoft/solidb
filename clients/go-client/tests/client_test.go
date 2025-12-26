package tests

import (
	"os"
	"strconv"
	"testing"

	"github.com/solisoft/solidb-go-client/solidb"
)

func TestClient(t *testing.T) {
	portStr := os.Getenv("SOLIDB_PORT")
	if portStr == "" {
		portStr = "6745"
	}
	port, _ := strconv.Atoi(portStr)

	client := solidb.NewClient("127.0.0.1", port)
	err := client.Connect()
	if err != nil {
		t.Fatalf("Failed to connect: %v", err)
	}
	defer client.Close()

	// Ping
	err = client.Ping()
	if err != nil {
		t.Errorf("Ping failed: %v", err)
	}

	dbName := "go_test_db"
	colName := "users"

	// Cleanup
	_ = client.DeleteDatabase(dbName)

	// Create DB
	err = client.CreateDatabase(dbName)
	if err != nil {
		t.Fatalf("CreateDatabase failed: %v", err)
	}

	// Create Collection
	err = client.CreateCollection(dbName, colName, nil)
	if err != nil {
		t.Fatalf("CreateCollection failed: %v", err)
	}

	// Insert
	doc := map[string]interface{}{
		"name": "Go",
		"ver":  1.21,
	}
	res, err := client.Insert(dbName, colName, doc, nil)
	if err != nil {
		t.Fatalf("Insert failed: %v", err)
	}

	key, ok := res["_key"].(string)
	if !ok {
		t.Fatalf("Missing _key in insert response")
	}

	// Get
	fetched, err := client.Get(dbName, colName, key)
	if err != nil {
		t.Fatalf("Get failed: %v", err)
	}
	if fetched["name"] != "Go" {
		t.Errorf("Expected name Go, got %v", fetched["name"])
	}

	// Update
	err = client.Update(dbName, colName, key, map[string]interface{}{"ver": 1.22}, true)
	if err != nil {
		t.Fatalf("Update failed: %v", err)
	}

	// Query
	results, err := client.Query(dbName, "FOR u IN users RETURN u", nil)
	if err != nil {
		t.Fatalf("Query failed: %v", err)
	}
	if len(results) == 0 {
		t.Errorf("Expected results, got 0")
	}

	// Transaction
	txID, err := client.BeginTransaction(dbName, nil)
	if err != nil {
		t.Fatalf("BeginTransaction failed: %v", err)
	}
	err = client.CommitTransaction(txID)
	if err != nil {
		t.Fatalf("CommitTransaction failed: %v", err)
	}

	// Delete
	err = client.Delete(dbName, colName, key)
	if err != nil {
		t.Fatalf("Delete failed: %v", err)
	}

	// Cleanup
	err = client.DeleteDatabase(dbName)
	if err != nil {
		t.Errorf("Final cleanup failed: %v", err)
	}
}
