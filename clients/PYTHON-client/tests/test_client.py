import pytest
import os
import time
from solidb import Client, ServerError, ConnectionError, AuthError

# Configuration
PORT = int(os.environ.get("SOLIDB_PORT", 6745))
DB_NAME = "python_test_db"
COLLECTION_NAME = "users"

@pytest.fixture(scope="module")
def client():
    c = Client(host="127.0.0.1", port=PORT)
    c.connect()
    try:
        c.auth("_system", "admin", "admin")
    except:
        pass # Ignore auth errors if server not enforcing
    yield c
    c.close()

def test_connection(client):
    assert client.ping() is True

def test_crud(client):
    # Cleanup
    try:
        client.delete_database(DB_NAME)
    except ServerError:
        pass
        
    client.create_database(DB_NAME)
    client.create_collection(DB_NAME, COLLECTION_NAME)
    
    # Insert
    doc = {"name": "Python", "version": 3.9}
    inserted = client.insert(DB_NAME, COLLECTION_NAME, doc)
    assert "_key" in inserted
    assert inserted["name"] == "Python"
    key = inserted["_key"]
    
    # Get
    fetched = client.get(DB_NAME, COLLECTION_NAME, key)
    assert fetched["name"] == "Python"
    
    # Update
    client.update(DB_NAME, COLLECTION_NAME, key, {"version": 3.11})
    fetched = client.get(DB_NAME, COLLECTION_NAME, key)
    assert fetched["version"] == 3.11
    
    # List
    docs = client.list_documents(DB_NAME, COLLECTION_NAME)
    assert len(docs) >= 1
    
    # Delete
    client.delete(DB_NAME, COLLECTION_NAME, key)
    
    with pytest.raises(ServerError):
        client.get(DB_NAME, COLLECTION_NAME, key)
        
    # Cleanup
    client.delete_database(DB_NAME)

def test_query(client):
    try:
        client.delete_database(DB_NAME)
    except: pass
    client.create_database(DB_NAME)
    client.create_collection(DB_NAME, "items")
    
    client.insert(DB_NAME, "items", {"val": 10})
    client.insert(DB_NAME, "items", {"val": 20})
    
    # Query
    res = client.query(DB_NAME, "FOR i IN items FILTER i.val > @threshold RETURN i", {"threshold": 15})
    assert isinstance(res, list)
    assert len(res) == 1
    assert res[0]["val"] == 20
    
    client.delete_database(DB_NAME)

def test_transaction(client):
    try:
        client.delete_database(DB_NAME)
    except: pass
    client.create_database(DB_NAME)
    client.create_collection(DB_NAME, "tx_col")
    
    tx_id = client.begin_transaction(DB_NAME)
    assert isinstance(tx_id, str) or isinstance(tx_id, int) # Depending on server implementation
    
    # We can't easily test isolation without parallel clients, but we can test API flow
    client.commit_transaction(tx_id)
    
    # Rollback flow
    tx_id_2 = client.begin_transaction(DB_NAME)
    client.rollback_transaction(tx_id_2)
    
    client.delete_database(DB_NAME)


def test_foreign_keys(client):
    db = "python_fk_db"
    try:
        client.delete_database(db)
    except ServerError:
        pass
    client.create_database(db)
    client.create_collection(db, "customers")
    client.create_collection(db, "orders")
    client.insert(db, "customers", {"_key": "c1", "name": "Ada"})
    client.insert(db, "orders", {"_key": "o1", "customer": "c1"})
    client.insert(db, "orders", {"_key": "o2", "customer": "ghost"})

    # A document that refers to a missing parent: the key is refused, and
    # the check says which.
    found = client.check_foreign_key(db, "orders", "customer", "customers")
    assert found["count"] == 1
    assert found["sample"][0]["_key"] == "o2"
    with pytest.raises(ServerError):
        client.create_foreign_key(db, "orders", "customer", "customers")

    client.delete(db, "orders", "o2")
    key = client.create_foreign_key(
        db, "orders", "customer", "customers", on_delete="cascade", name="fk_customer")
    assert key["name"] == "fk_customer"
    assert key["on_delete"] == "cascade"
    assert client.list_foreign_keys(db, "orders")["foreign_keys"][0]["field"] == "customer"
    assert client.list_foreign_keys(db, "customers")["referenced_by"][0]["collection"] == "orders"

    with pytest.raises(ServerError):
        client.insert(db, "orders", {"_key": "o3", "customer": "nobody"})
    client.delete(db, "customers", "c1")
    assert client.query(db, "FOR o IN orders RETURN o._key") == []

    client.delete_foreign_key(db, "orders", "fk_customer")
    assert client.list_foreign_keys(db, "orders")["foreign_keys"] == []
    client.delete_database(db)
