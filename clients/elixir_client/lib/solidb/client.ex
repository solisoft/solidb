defmodule SoliDB.Client do
  @moduledoc """
  Elixir client for SoliDB.
  """

  @magic_header "solidb-drv-v1\0"
  @max_message_size 16 * 1024 * 1024
  @default_pool_size 4

  # Inside a transaction the server stages these when they arrive wrapped in a
  # `transaction_command` on the connection that began it (a read-only query
  # runs on committed data; a writing one is refused there).
  @staged_in_transaction ~w(insert update delete query)
  # These cannot be staged: refused rather than applied outside the transaction.
  @refused_in_transaction ~w(bulk_insert batch begin_transaction)

  # `tx_id`: the transaction this client value's commands belong to, set by
  # `in_transaction/2`.
  defstruct [:host, :port, :pool, :pool_index, :database, :tx_id]

  def connect(host \\ "127.0.0.1", port \\ 6745, pool_size \\ @default_pool_size) do
    connect_with_pool(host, port, pool_size)
  end

  defp connect_with_pool(host, port, pool_size) do
    result =
      Enum.reduce(1..pool_size, [], fn _, acc ->
        case :gen_tcp.connect(String.to_charlist(host), port, [:binary, active: false, packet: 0, nodelay: true]) do
          {:ok, socket} ->
            :ok = :gen_tcp.send(socket, @magic_header)
            [socket | acc]
          {:error, reason} ->
            Enum.each(acc, &:gen_tcp.close/1)
            {:error, "Failed to connect: #{inspect(reason)}"}
        end
      end)

    case result do
      sockets when is_list(sockets) and length(sockets) == pool_size ->
        {:ok, %__MODULE__{host: host, port: port, pool: Enum.reverse(sockets), pool_index: 0, database: nil}}
      {:error, _} = error ->
        error
      _ ->
        {:error, "Failed to establish all connections"}
    end
  end

  def close(%__MODULE__{pool: pool}) do
    Enum.each(pool, &:gen_tcp.close/1)
  end

  def use_database(%__MODULE__{} = client, database) do
    %{client | database: database}
  end

  def get_database(%__MODULE__{database: database}), do: database

  def ping(client), do: send_command(client, "ping", %{})

  # Authentication is per connection: every pooled socket logs in.
  def auth(client, database, username, password) do
    auth_all(client, %{database: database, username: username, password: password})
  end

  def auth_with_api_key(client, database, api_key) do
    auth_all(client, %{database: database, username: "", password: "", api_key: api_key})
  end

  defp auth_all(client, args) do
    Enum.reduce_while(0..(length(client.pool) - 1), {:ok, nil}, fn idx, _ ->
      case send_command(%{client | pool_index: idx, tx_id: nil}, "auth", args) do
        {:ok, _} = ok -> {:cont, ok}
        error -> {:halt, error}
      end
    end)
  end

  def list_databases(client), do: send_command(client, "list_databases", %{})
  def create_database(client, name), do: send_command(client, "create_database", %{name: name})
  def delete_database(client, name), do: send_command(client, "delete_database", %{name: name})

  def list_collections(client, database),
    do: send_command(client, "list_collections", %{database: database})

  def create_collection(client, database, name, type \\ nil) do
    args = %{database: database, name: name}
    args = if type, do: Map.put(args, :type, type), else: args
    send_command(client, "create_collection", args)
  end

  def delete_collection(client, database, name),
    do: send_command(client, "delete_collection", %{database: database, name: name})

  def insert(client, database, collection, document, key \\ nil) do
    args = %{database: database, collection: collection, document: document}
    args = if key, do: Map.put(args, :key, key), else: args
    send_command(client, "insert", args)
  end

  def get(client, database, collection, key) do
    send_command(client, "get", %{database: database, collection: collection, key: key})
  end

  def update(client, database, collection, key, document, merge \\ true) do
    send_command(client, "update", %{
      database: database,
      collection: collection,
      key: key,
      document: document,
      merge: merge
    })
  end

  def delete(client, database, collection, key) do
    send_command(client, "delete", %{database: database, collection: collection, key: key})
  end

  def list_documents(client, database, collection, limit \\ 50, offset \\ 0) do
    send_command(client, "list", %{
      database: database,
      collection: collection,
      limit: limit,
      offset: offset
    })
  end

  def query(client, database, sdbql, bind_vars \\ %{}) do
    send_command(client, "query", %{database: database, sdbql: sdbql, bind_vars: bind_vars})
  end

  @doc """
  Open a transaction and return `{:ok, tx_id}`.

  The writes of the transaction are made through `in_transaction/2` (or
  `transaction/3`), on the same socket: the server keeps a driver transaction
  per connection.
  """
  def begin_transaction(client, database, isolation_level \\ "read_committed") do
    send_command(%{client | tx_id: nil}, "begin_transaction", %{
      database: database,
      isolation_level: isolation_level
    })
  end

  def commit_transaction(client, tx_id) do
    send_command(%{client | tx_id: nil}, "commit_transaction", %{tx_id: tx_id})
  end

  def rollback_transaction(client, tx_id) do
    send_command(%{client | tx_id: nil}, "rollback_transaction", %{tx_id: tx_id})
  end

  @doc """
  The client, with its commands made inside transaction `tx_id`.

  `insert`, `update` and `delete` made through it are staged in the
  transaction and applied at commit; a read-only `query` runs on committed data
  (it does not see the transaction's own writes); a query that writes and
  `begin_transaction` are refused. `get` is not part of the transaction: it
  reads committed data. Writes through the plain client are not part of it
  either: they used to be the only way, and applied at once.

      {:ok, tx_id} = SoliDB.Client.begin_transaction(client, "mydb")
      tx = SoliDB.Client.in_transaction(client, tx_id)
      {:ok, _} = SoliDB.Client.insert(tx, "mydb", "accounts", %{"balance" => 1000})
      {:ok, _} = SoliDB.Client.commit_transaction(client, tx_id)
  """
  def in_transaction(%__MODULE__{} = client, tx_id), do: %{client | tx_id: tx_id}

  @doc """
  Run `fun` with a client in a new transaction: commit when it returns
  `{:ok, value}`, roll back when it returns `{:error, reason}` or raises.
  """
  def transaction(client, database, fun, isolation_level \\ "read_committed") do
    with {:ok, tx_id} <- begin_transaction(client, database, isolation_level) do
      try do
        case fun.(in_transaction(client, tx_id)) do
          {:ok, value} ->
            with {:ok, _} <- commit_transaction(client, tx_id), do: {:ok, value}

          {:error, _} = error ->
            rollback_transaction(client, tx_id)
            error
        end
      rescue
        e ->
          rollback_transaction(client, tx_id)
          reraise e, __STACKTRACE__
      end
    end
  end

  # One client value always talks on the same socket: `pool_index` is part of
  # the value, and the advanced index was never handed back, so the "round
  # robin" this used to do never moved.
  def send_command(client, cmd, args) do
    payload = Map.put(args, :cmd, cmd)

    cond do
      client.tx_id != nil and cmd in @staged_in_transaction ->
        send_payload(client, %{cmd: "transaction_command", tx_id: client.tx_id, command: payload})

      client.tx_id != nil and cmd in @refused_in_transaction ->
        {:error, "#{cmd} cannot run inside a transaction: commit or roll back first"}

      true ->
        send_payload(client, payload)
    end
  end

  defp send_payload(client, payload) do
    socket = Enum.at(client.pool, client.pool_index || 0)
    # A binary, not the iodata Msgpax returns by default: byte_size/1 below
    # raised on every command.
    {:ok, data} = Msgpax.pack(payload, iodata: false)
    len = byte_size(data)
    :ok = :gen_tcp.send(socket, <<len::big-integer-size(32), data::binary>>)
    receive_response(socket)
  end

  defp receive_response(socket) do
    case :gen_tcp.recv(socket, 4) do
      {:ok, <<len::big-integer-size(32)>>} ->
        if len > @max_message_size do
          {:error, "Message too large"}
        else
          case :gen_tcp.recv(socket, len) do
            {:ok, data} ->
              case Msgpax.unpack(data) do
                {:ok, response} -> normalize_response(response)
                error -> error
              end

            {:error, reason} ->
              {:error, "Recv body failed: #{inspect(reason)}"}
          end
        end

      {:error, reason} ->
        {:error, "Recv header failed: #{inspect(reason)}"}
    end
  end

  defp normalize_response(response) do
    case response do
      ["ok", body] ->
        {:ok, body}

      ["pong", body] ->
        {:ok, body}

      ["error", body] ->
        {:error, error_message(body)}

      # What the server sends: `{status, data, count, tx_id, error}`. It was
      # returned as it was, so every error came back as `{:ok, map}`.
      %{"status" => "error"} = m ->
        {:error, error_message(Map.get(m, "error"))}

      %{"status" => status} = m when status in ["ok", "pong"] ->
        cond do
          Map.has_key?(m, "data") -> {:ok, m["data"]}
          Map.has_key?(m, "tx_id") -> {:ok, m["tx_id"]}
          Map.has_key?(m, "count") -> {:ok, m["count"]}
          true -> {:ok, nil}
        end

      other ->
        {:ok, other}
    end
  end

  # The server's error as text: a string, or %{"Kind" => "message"}.
  defp error_message(body) when is_binary(body), do: body
  defp error_message(body) when is_map(body), do: body |> Map.values() |> List.first() |> to_string()
  defp error_message(body), do: inspect(body)
end
