# Transactions over the driver, against a running server (card #793):
#
#     SOLIDB_PORT=6745 mix test
#
# Writes made "in" a transaction used to go out as plain commands, applied at
# once and kept after a rollback; every server error came back as {:ok, map}.
defmodule SoliDB.TransactionTest do
  use ExUnit.Case, async: false

  alias SoliDB.Client

  @db "elixir_tx_test_db"

  defp connected do
    port = String.to_integer(System.get_env("SOLIDB_PORT", "6745"))
    {:ok, c} = Client.connect("127.0.0.1", port)

    {:ok, _} =
      Client.auth(c, "_system", System.get_env("SOLIDB_USER", "admin"),
        System.get_env("SOLIDB_PASSWORD", "admin"))

    c
  end

  defp n(c, key) do
    case Client.get(c, @db, "docs", key) do
      {:ok, %{"n" => n}} -> n
      _ -> nil
    end
  end

  setup do
    c = connected()
    observer = connected()
    Client.delete_database(c, @db)
    {:ok, _} = Client.create_database(c, @db)
    {:ok, _} = Client.create_collection(c, @db, "docs")
    {:ok, _} = Client.insert(c, @db, "docs", %{"n" => 1}, "base")
    {:ok, _} = Client.insert(c, @db, "docs", %{"n" => 1}, "gone")

    # on_exit runs in another process; the test's sockets close with the
    # test's own process, so the cleanup opens its own.
    on_exit(fn ->
      cleanup = connected()
      Client.delete_database(cleanup, @db)
      Client.close(cleanup)
    end)

    %{c: c, observer: observer}
  end

  test "errors come back as errors", %{c: c} do
    assert {:error, _} = Client.get(c, @db, "docs", "missing")
  end

  test "a rollback leaves nothing, and nothing shows before it", %{c: c, observer: o} do
    {:ok, tx_id} = Client.begin_transaction(c, @db)
    assert is_binary(tx_id)
    tx = Client.in_transaction(c, tx_id)
    {:ok, _} = Client.insert(tx, @db, "docs", %{"n" => 7}, "new")
    {:ok, _} = Client.update(tx, @db, "docs", "base", %{"n" => 2})
    {:ok, _} = Client.delete(tx, @db, "docs", "gone")
    assert n(o, "new") == nil
    assert n(o, "base") == 1
    {:ok, _} = Client.rollback_transaction(c, tx_id)
    assert n(o, "new") == nil
    assert n(o, "base") == 1
    assert n(o, "gone") == 1
  end

  test "transaction/3 commits every write at once", %{c: c, observer: o} do
    assert {:ok, :done} =
             Client.transaction(c, @db, fn tx ->
               {:ok, _} = Client.insert(tx, @db, "docs", %{"n" => 3}, "a")
               {:ok, _} = Client.update(tx, @db, "docs", "base", %{"n" => 2})
               {:ok, _} = Client.delete(tx, @db, "docs", "gone")
               {:ok, [1]} = Client.query(tx, @db, "FOR d IN docs FILTER d._key == 'base' RETURN d.n")
               nil = n(o, "a")
               {:ok, :done}
             end)

    assert n(o, "a") == 3
    assert n(o, "base") == 2
    assert n(o, "gone") == nil
  end

  test "transaction/3 rolls back on an error or a raise", %{c: c, observer: o} do
    assert {:error, :nope} =
             Client.transaction(c, @db, fn tx ->
               {:ok, _} = Client.insert(tx, @db, "docs", %{"n" => 4}, "b")
               {:error, :nope}
             end)

    assert_raise RuntimeError, fn ->
      Client.transaction(c, @db, fn tx ->
        {:ok, _} = Client.insert(tx, @db, "docs", %{"n" => 5}, "c")
        raise "boom"
      end)
    end

    assert n(o, "b") == nil
    assert n(o, "c") == nil
  end

  test "what cannot be staged is refused", %{c: c, observer: o} do
    {:ok, tx_id} = Client.begin_transaction(c, @db)
    tx = Client.in_transaction(c, tx_id)
    assert {:error, msg} = Client.query(tx, @db, "INSERT {_key: 'q', n: 1} INTO docs")
    assert msg =~ "cannot run inside a transaction"
    assert {:error, msg} = Client.send_command(tx, "batch", %{commands: []})
    assert msg =~ "cannot run inside a transaction"
    {:ok, _} = Client.rollback_transaction(c, tx_id)
    assert n(o, "q") == nil
  end
end
