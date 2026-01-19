defmodule SoliDB.Client do
  @moduledoc """
  Elixir client for SoliDB.
  """

  @magic_header "solidb-drv-v1\0"
  @max_message_size 16 * 1024 * 1024

  defstruct [:host, :port, :socket, :database]

  @doc """
  Sets the database context for the client.
  """
  def use_database(%__MODULE__{} = client, database) do
    %{client | database: database}
  end

  @doc """
  Gets the current database from the client.
  """
  def get_database(%__MODULE__{database: database}), do: database

  def connect(host \\ "127.0.0.1", port \\ 6745) do
    case :gen_tcp.connect(String.to_charlist(host), port, [:binary, active: false, packet: 0]) do
      {:ok, socket} ->
        :ok = :gen_tcp.send(socket, @magic_header)
        {:ok, %__MODULE__{host: host, port: port, socket: socket}}

      {:error, reason} ->
        {:error, "Failed to connect: #{inspect(reason)}"}
    end
  end

  def close(%__MODULE__{socket: socket}) do
    :gen_tcp.close(socket)
  end

  # --- API Methods ---

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

  # Database
  def list_databases(client), do: send_command(client, "list_databases", %{})
  def create_database(client, name), do: send_command(client, "create_database", %{name: name})
  def delete_database(client, name), do: send_command(client, "delete_database", %{name: name})

  # Collection
  def list_collections(client, database),
    do: send_command(client, "list_collections", %{database: database})

  def create_collection(client, database, name, type \\ nil) do
    args = %{database: database, name: name}
    args = if type, do: Map.put(args, :type, type), else: args
    send_command(client, "create_collection", args)
  end

  def delete_collection(client, database, name),
    do: send_command(client, "delete_collection", %{database: database, name: name})

  # Document
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

  # Query
  def query(client, database, sdbql, bind_vars \\ %{}) do
    send_command(client, "query", %{database: database, sdbql: sdbql, bind_vars: bind_vars})
  end

  # Transactions
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

  # --- Command Interface (public for sub-client modules) ---

  @doc """
  Sends a command to the server. Used internally and by sub-client modules.
  """
  def send_command(client, cmd, args) do
    payload = Map.put(args, :cmd, cmd)
    {:ok, data} = Msgpax.pack(payload)

    # Prefix with 4-byte big-endian length
    len = byte_size(data)
    # wait, actually <<len::32>>
    :ok = :gen_tcp.send(client.socket, <<len::inline(), 32, data::binary>>)
    # Correct bitstring for big-endian 32-bit:
    :ok = :gen_tcp.send(client.socket, <<len::big-integer-size(32), data::binary>>)

    receive_response(client)
  end

  defp receive_response(client) do
    case :gen_tcp.recv(client.socket, 4) do
      {:ok, <<len::big-integer-size(32)>>} ->
        if len > @max_message_size do
          {:error, "Message too large"}
        else
          case :gen_tcp.recv(client.socket, len) do
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
