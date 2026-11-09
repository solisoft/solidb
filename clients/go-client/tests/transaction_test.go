package tests

import (
	"errors"
	"strconv"
	"strings"
	"testing"

	"github.com/solisoft/solidb-go-client/solidb"
)

// Transactions over the driver, against a running server (card #793):
//
//	SOLIDB_PORT=6745 go test ./tests/ -run Transaction
//
// The client used to send a transaction's writes as plain commands, applied
// at once and kept after a rollback.

const txDB = "go_tx_test_db"

func connected(t *testing.T) *solidb.Client {
	port, _ := strconv.Atoi(env("SOLIDB_PORT", "6745"))
	c := solidb.NewClient("127.0.0.1", port)
	if err := c.Connect(); err != nil {
		t.Fatalf("connect: %v", err)
	}
	if err := c.Auth("_system", env("SOLIDB_USER", "admin"), env("SOLIDB_PASSWORD", "admin")); err != nil {
		t.Fatalf("auth: %v", err)
	}
	return c
}

// n reads `n` of a document as another client sees it; -1 when absent.
func n(t *testing.T, c *solidb.Client, key string) float64 {
	doc, err := c.Get(txDB, "docs", key)
	if err != nil {
		return -1
	}
	// MessagePack integers decode to the smallest Go integer type that fits.
	switch v := doc["n"].(type) {
	case int8:
		return float64(v)
	case int16:
		return float64(v)
	case int32:
		return float64(v)
	case int64:
		return float64(v)
	case uint8:
		return float64(v)
	case uint16:
		return float64(v)
	case uint32:
		return float64(v)
	case uint64:
		return float64(v)
	case float64:
		return v
	}
	t.Fatalf("%s: n is %T", key, doc["n"])
	return 0
}

func TestTransactions(t *testing.T) {
	c, observer := connected(t), connected(t)
	defer c.Close()
	defer observer.Close()
	_ = c.DeleteDatabase(txDB)
	if err := c.CreateDatabase(txDB); err != nil {
		t.Fatalf("create database: %v", err)
	}
	defer c.DeleteDatabase(txDB)
	if err := c.CreateCollection(txDB, "docs", nil); err != nil {
		t.Fatalf("create collection: %v", err)
	}
	for _, k := range []string{"base", "gone"} {
		key := k
		if _, err := c.Insert(txDB, "docs", map[string]interface{}{"n": 1}, &key); err != nil {
			t.Fatalf("insert %s: %v", k, err)
		}
	}
	newKey, aKey := "new", "a"

	// A rollback leaves nothing, and nothing shows before it.
	tx, err := c.BeginTransaction(txDB, nil)
	if err != nil {
		t.Fatalf("begin: %v", err)
	}
	if !c.InTransaction() {
		t.Fatal("InTransaction false after begin")
	}
	if _, err := c.Insert(txDB, "docs", map[string]interface{}{"n": 7}, &newKey); err != nil {
		t.Fatalf("insert in tx: %v", err)
	}
	if err := c.Update(txDB, "docs", "base", map[string]interface{}{"n": 2}, true); err != nil {
		t.Fatalf("update in tx: %v", err)
	}
	if err := c.Delete(txDB, "docs", "gone"); err != nil {
		t.Fatalf("delete in tx: %v", err)
	}
	if got := n(t, observer, "new"); got != -1 {
		t.Errorf("staged insert visible before the end: %v", got)
	}
	if got := n(t, observer, "base"); got != 1 {
		t.Errorf("staged update visible before the end: %v", got)
	}
	if err := c.RollbackTransaction(tx); err != nil {
		t.Fatalf("rollback: %v", err)
	}
	if got := n(t, observer, "new"); got != -1 {
		t.Errorf("insert survived the rollback: %v", got)
	}
	if got := n(t, observer, "base"); got != 1 {
		t.Errorf("update survived the rollback: %v", got)
	}
	if got := n(t, observer, "gone"); got != 1 {
		t.Errorf("delete survived the rollback: %v", got)
	}

	// A commit applies every write at once.
	tx, err = c.BeginTransaction(txDB, nil)
	if err != nil {
		t.Fatalf("begin: %v", err)
	}
	if _, err := c.Insert(txDB, "docs", map[string]interface{}{"n": 3}, &aKey); err != nil {
		t.Fatalf("insert in tx: %v", err)
	}
	if err := c.Update(txDB, "docs", "base", map[string]interface{}{"n": 2}, true); err != nil {
		t.Fatalf("update in tx: %v", err)
	}
	rows, err := c.Query(txDB, "FOR d IN docs FILTER d._key == 'base' RETURN d.n", nil)
	if err != nil || len(rows) != 1 {
		t.Fatalf("read-only query in tx: %v %v", rows, err)
	}
	if got := n(t, observer, "a"); got != -1 {
		t.Errorf("staged insert visible before the commit: %v", got)
	}
	if err := c.CommitTransaction(tx); err != nil {
		t.Fatalf("commit: %v", err)
	}
	if got := n(t, observer, "a"); got != 3 {
		t.Errorf("committed insert: %v", got)
	}
	if got := n(t, observer, "base"); got != 2 {
		t.Errorf("committed update: %v", got)
	}

	// What cannot be staged is refused.
	tx, err = c.BeginTransaction(txDB, nil)
	if err != nil {
		t.Fatalf("begin: %v", err)
	}
	if _, err := c.Query(txDB, "INSERT {_key: 'q', n: 1} INTO docs", nil); err == nil ||
		!strings.Contains(err.Error(), "cannot run inside a transaction") {
		t.Errorf("writing query in tx: %v", err)
	}
	var txErr *solidb.TransactionError
	if _, err := c.BeginTransaction(txDB, nil); !errors.As(err, &txErr) {
		t.Errorf("second begin: %v", err)
	}
	if err := c.RollbackTransaction(tx); err != nil {
		t.Fatalf("rollback: %v", err)
	}
	if err := c.CommitTransaction(tx); !errors.As(err, &txErr) {
		t.Errorf("commit after rollback: %v", err)
	}
	if got := n(t, observer, "q"); got != -1 {
		t.Errorf("refused query wrote: %v", got)
	}
}
