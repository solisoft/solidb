"""Transactions over the driver, against a running server (card #793).

    SOLIDB_PORT=6745 pytest tests/test_transactions.py

The client used to send a transaction's writes as plain commands, applied at
once and kept after a rollback; `begin_transaction` returned None, and its
default pool of four sockets lost the login on three of them.
"""
import os

import pytest

from solidb import Client, ServerError, TransactionError

PORT = int(os.environ.get("SOLIDB_PORT", 6745))
USER = os.environ.get("SOLIDB_USER", "admin")
PASSWORD = os.environ.get("SOLIDB_PASSWORD", "admin")
DB = "python_tx_test_db"


def make_client():
    c = Client(host="127.0.0.1", port=PORT)  # the default pool of four
    c.connect()
    c.auth("_system", USER, PASSWORD)
    return c


@pytest.fixture(scope="module")
def clients():
    c, observer = make_client(), make_client()
    try:
        c.delete_database(DB)
    except ServerError:
        pass
    c.create_database(DB)
    c.create_collection(DB, "docs")
    yield c, observer
    c.delete_database(DB)
    c.close()
    observer.close()


@pytest.fixture(autouse=True)
def fresh_docs(clients):
    c, _ = clients
    c.query(DB, "FOR d IN docs REMOVE d IN docs")
    c.insert(DB, "docs", {"n": 1}, key="base")
    c.insert(DB, "docs", {"n": 1}, key="gone")


def n(c, key):
    try:
        return c.get(DB, "docs", key)["n"]
    except ServerError:
        return None


def test_a_rollback_leaves_nothing_and_nothing_shows_before_it(clients):
    c, observer = clients
    tx = c.begin_transaction(DB)
    assert isinstance(tx, str) and tx
    assert c.in_transaction
    c.insert(DB, "docs", {"n": 7}, key="new")
    c.update(DB, "docs", "base", {"n": 2})
    c.delete(DB, "docs", "gone")
    assert n(observer, "new") is None
    assert n(observer, "base") == 1
    c.rollback_transaction(tx)
    assert not c.in_transaction
    assert n(observer, "new") is None
    assert n(observer, "base") == 1
    assert n(observer, "gone") == 1


def test_a_commit_applies_every_write_at_once(clients):
    c, observer = clients
    tx = c.begin_transaction(DB)
    c.insert(DB, "docs", {"n": 3}, key="a")
    # Reads outside the transaction go out on the other pooled sockets.
    assert c.get(DB, "docs", "base")["n"] == 1
    c.update(DB, "docs", "base", {"n": 2})
    c.delete(DB, "docs", "gone")
    assert c.query(DB, "FOR d IN docs FILTER d._key == 'base' RETURN d.n") == [1]
    assert n(observer, "a") is None
    c.commit_transaction(tx)
    assert n(observer, "a") == 3
    assert n(observer, "base") == 2
    assert n(observer, "gone") is None


def test_a_writing_query_is_staged_and_a_second_begin_refused(clients):
    c, observer = clients
    tx = c.begin_transaction(DB)
    c.query(DB, "INSERT {_key: 'q', n: 1} INTO docs")
    assert n(observer, "q") is None
    with pytest.raises(TransactionError):
        c.begin_transaction(DB)
    c.rollback_transaction(tx)
    assert n(observer, "q") is None
    with pytest.raises(TransactionError):
        c.commit_transaction(tx)


def test_every_pooled_socket_is_logged_in(clients):
    c, _ = clients
    for _ in range(8):
        assert DB in c.list_databases()
