package solidb

import (
	"bufio"
	"encoding/binary"
	"fmt"
	"io"
	"net"
	"sync"
	"time"

	"github.com/vmihailenco/msgpack/v5"
)

const (
	MagicHeader    = "solidb-drv-v1\x00"
	MaxMessageSize = 16 * 1024 * 1024
)

type Client struct {
	addr string
	conn net.Conn
	mu   sync.Mutex
	br   *bufio.Reader
	bw   *bufio.Writer
}

func NewClient(host string, port int) *Client {
	return &Client{
		addr: fmt.Sprintf("%s:%d", host, port),
	}
}

func (c *Client) Connect() error {
	c.mu.Lock()
	defer c.mu.Unlock()

	if c.conn != nil {
		return nil
	}

	conn, err := net.DialTimeout("tcp", c.addr, 5*time.Second)
	if err != nil {
		return &ConnectionError{SoliDBError{Message: err.Error()}}
	}

	_, err = conn.Write([]byte(MagicHeader))
	if err != nil {
		conn.Close()
		return &ConnectionError{SoliDBError{Message: fmt.Sprintf("failed to send magic header: %v", err)}}
	}

	c.conn = conn
	c.br = bufio.NewReader(conn)
	c.bw = bufio.NewWriter(conn)
	return nil
}

func (c *Client) Close() error {
	c.mu.Lock()
	defer c.mu.Unlock()

	if c.conn == nil {
		return nil
	}

	err := c.conn.Close()
	c.conn = nil
	return err
}

func (c *Client) sendCommand(cmd string, args map[string]interface{}) (interface{}, error) {
	if err := c.Connect(); err != nil {
		return nil, err
	}

	c.mu.Lock()
	defer c.mu.Unlock()

	payload := make(map[string]interface{})
	payload["cmd"] = cmd
	for k, v := range args {
		payload[k] = v
	}

	data, err := msgpack.Marshal(payload)
	if err != nil {
		return nil, &ProtocolError{SoliDBError{Message: fmt.Sprintf("marshal error: %v", err)}}
	}

	// Write length prefix (4 bytes BE)
	lenBuf := make([]byte, 4)
	binary.BigEndian.PutUint32(lenBuf, uint32(len(data)))
	if _, err := c.bw.Write(lenBuf); err != nil {
		return nil, &ConnectionError{SoliDBError{Message: err.Error()}}
	}

	// Write payload
	if _, err := c.bw.Write(data); err != nil {
		return nil, &ConnectionError{SoliDBError{Message: err.Error()}}
	}

	if err := c.bw.Flush(); err != nil {
		return nil, &ConnectionError{SoliDBError{Message: err.Error()}}
	}

	return c.receiveResponse()
}

func (c *Client) receiveResponse() (interface{}, error) {
	lenBuf := make([]byte, 4)
	if _, err := io.ReadFull(c.br, lenBuf); err != nil {
		return nil, &ConnectionError{SoliDBError{Message: err.Error()}}
	}

	length := binary.BigEndian.Uint32(lenBuf)
	if length > MaxMessageSize {
		return nil, &ProtocolError{SoliDBError{Message: fmt.Sprintf("message too large: %d", length)}}
	}

	data := make([]byte, length)
	if _, err := io.ReadFull(c.br, data); err != nil {
		return nil, &ConnectionError{SoliDBError{Message: err.Error()}}
	}

	var res interface{}
	if err := msgpack.Unmarshal(data, &res); err != nil {
		return nil, &ProtocolError{SoliDBError{Message: fmt.Sprintf("unmarshal error: %v", err)}}
	}

	// Handle [status, body] tuple
	if slice, ok := res.([]interface{}); ok && len(slice) >= 1 {
		status, ok := slice[0].(string)
		if !ok {
			return res, nil
		}

		var body interface{}
		if len(slice) > 1 {
			body = slice[1]
		}

		if status == "ok" || status == "pong" {
			return body, nil
		}

		if status == "error" {
			msg := "unknown error"
			if m, ok := body.(string); ok {
				msg = m
			} else if m, ok := body.(map[string]interface{}); ok {
				for _, v := range m {
					msg = fmt.Sprintf("%v", v)
					break
				}
			}
			return nil, &ServerError{SoliDBError{Message: msg}}
		}
	}

	return res, nil
}

// API Methods

func (c *Client) Ping() error {
	_, err := c.sendCommand("ping", nil)
	return err
}

func (c *Client) Auth(database, username, password string) error {
	_, err := c.sendCommand("auth", map[string]interface{}{
		"database": database,
		"username": username,
		"password": password,
	})
	return err
}

func (c *Client) ListDatabases() ([]string, error) {
	res, err := c.sendCommand("list_databases", nil)
	if err != nil {
		return nil, err
	}
	if slice, ok := res.([]interface{}); ok {
		out := make([]string, len(slice))
		for i, v := range slice {
			out[i] = v.(string)
		}
		return out, nil
	}
	return nil, nil
}

func (c *Client) CreateDatabase(name string) error {
	_, err := c.sendCommand("create_database", map[string]interface{}{"name": name})
	return err
}

func (c *Client) DeleteDatabase(name string) error {
	_, err := c.sendCommand("delete_database", map[string]interface{}{"name": name})
	return err
}

func (c *Client) ListCollections(database string) ([]string, error) {
	res, err := c.sendCommand("list_collections", map[string]interface{}{"database": database})
	if err != nil {
		return nil, err
	}
	if slice, ok := res.([]interface{}); ok {
		out := make([]string, len(slice))
		for i, v := range slice {
			out[i] = v.(string)
		}
		return out, nil
	}
	return nil, nil
}

func (c *Client) CreateCollection(database, name string, colType *string) error {
	args := map[string]interface{}{"database": database, "name": name}
	if colType != nil {
		args["type"] = *colType
	}
	_, err := c.sendCommand("create_collection", args)
	return err
}

func (c *Client) DeleteCollection(database, name string) error {
	_, err := c.sendCommand("delete_collection", map[string]interface{}{"database": database, "name": name})
	return err
}

func (c *Client) Insert(database, collection string, document interface{}, key *string) (map[string]interface{}, error) {
	args := map[string]interface{}{
		"database":   database,
		"collection": collection,
		"document":   document,
	}
	if key != nil {
		args["key"] = *key
	}
	res, err := c.sendCommand("insert", args)
	if err != nil {
		return nil, err
	}
	if m, ok := res.(map[string]interface{}); ok {
		return m, nil
	}
	return nil, nil
}

func (c *Client) Get(database, collection, key string) (map[string]interface{}, error) {
	res, err := c.sendCommand("get", map[string]interface{}{
		"database":   database,
		"collection": collection,
		"key":        key,
	})
	if err != nil {
		return nil, err
	}
	if m, ok := res.(map[string]interface{}); ok {
		return m, nil
	}
	return nil, nil
}

func (c *Client) Update(database, collection, key string, document interface{}, merge bool) error {
	_, err := c.sendCommand("update", map[string]interface{}{
		"database":   database,
		"collection": collection,
		"key":        key,
		"document":   document,
		"merge":      merge,
	})
	return err
}

func (c *Client) Delete(database, collection, key string) error {
	_, err := c.sendCommand("delete", map[string]interface{}{
		"database":   database,
		"collection": collection,
		"key":        key,
	})
	return err
}

func (c *Client) List(database, collection string, limit, offset int) ([]interface{}, error) {
	res, err := c.sendCommand("list", map[string]interface{}{
		"database":   database,
		"collection": collection,
		"limit":      limit,
		"offset":     offset,
	})
	if err != nil {
		return nil, err
	}
	if slice, ok := res.([]interface{}); ok {
		return slice, nil
	}
	return nil, nil
}

func (c *Client) Query(database, sdbql string, bindVars map[string]interface{}) ([]interface{}, error) {
	args := map[string]interface{}{
		"database": database,
		"sdbql":    sdbql,
	}
	if bindVars != nil {
		args["bind_vars"] = bindVars
	}
	res, err := c.sendCommand("query", args)
	if err != nil {
		return nil, err
	}
	if slice, ok := res.([]interface{}); ok {
		return slice, nil
	}
	return nil, nil
}

func (c *Client) BeginTransaction(database string, isolationLevel *string) (interface{}, error) {
	args := map[string]interface{}{"database": database}
	if isolationLevel != nil {
		args["isolation_level"] = *isolationLevel
	}
	return c.sendCommand("begin_transaction", args)
}

func (c *Client) CommitTransaction(txID interface{}) error {
	_, err := c.sendCommand("commit_transaction", map[string]interface{}{"tx_id": txID})
	return err
}

func (c *Client) RollbackTransaction(txID interface{}) error {
	_, err := c.sendCommand("rollback_transaction", map[string]interface{}{"tx_id": txID})
	return err
}
