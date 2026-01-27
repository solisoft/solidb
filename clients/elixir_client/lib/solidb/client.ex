defmodule SoliDB.Client do
  @moduledoc """
  Elixir client for SoliDB.
  """

  @magic_header "solidb-drv-v1\0"
  @max_message_size 16 * 1024 * 1024
  @default_pool_size 4

  defstruct [:host, :port, :pool, :pool_index, :database]

  def connect(host \\ "127.0.0.1", port \\ 6745, pool_size \\ @default_pool_size) do
    connect_with_pool(host, port, pool_size)
  end

  defp connect_with_pool(host, port, pool_size) do
    sockets =
      Enum.map(1..pool_size, fn _ ->
        case :gen_tcp.connect(String.to_charlist(host), port, [
               :binary,
               active: false,
               packet: 0,
               nodelay: true
             ]) do
          {:ok, socket} ->
            :ok = :gen_tcp.send(socket, @magic_header)
            socket

          {:error, reason} ->
            Enum.each(List.delete(sockets, nil), &:gen_tcp.close/1)
            {:error, "Failed to connect: #{inspect(reason)}"}
        end
      end)

    if Enum.all?(sockets, &is_port(&1)) do
      {:ok, %__MODULE__{host: host, port: port, pool: sockets, pool_index: 0, database: nil}}
    else
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

  def auth(client, database, username, password) do
    send_command(client, "auth", %{database: database, username: username, password: password})
  end

  def auth_with_api_key(client, database, api_key) do
    send_command(client, "auth", %{
      database: database,
      username: "",
      password: "",
      api_key: api_key
    })
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

  def begin_transaction(client, database, isolation_level \\ "read_committed") do
    send_command(client, "begin_transaction", %{
      database: database,
      isolation_level: isolation_level
    })
  end

  def commit_transaction(client, tx_id) do
    send_command(client, "commit_transaction", %{tx_id: tx_id})
  end

  def rollback_transaction(client, tx_id) do
    send_command(client, "rollback_transaction", %{tx_id: tx_id})
  end

  defp get_next_socket(client) do
    idx = client.pool_index
    socket = Enum.at(client.pool, idx)
    new_idx = rem(idx + 1, length(client.pool))
    %{client | pool_index: new_idx}
  end

  def send_command(client, cmd, args) do
    client = get_next_socket(client)
    payload = Map.put(args, :cmd, cmd)
    {:ok, data} = Msgpax.pack(payload)

    len = byte_size(data)

    :ok =
      :gen_tcp.send(
        Enum.at(client.pool, client.pool_index - 1),
        <<len::big-integer-size(32), data::binary>>
      )

    receive_response(client)
  end

  defp receive_response(client) do
    socket = Enum.at(client.pool, client.pool_index - 1)

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
        msg =
          case body do
            m when is_map(m) -> List.first(Map.values(m)) || "Unknown error"
            m -> m
          end

        {:error, msg}

      other ->
        {:ok, other}
    end
  end
end
