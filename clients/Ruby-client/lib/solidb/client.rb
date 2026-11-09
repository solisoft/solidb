require 'socket'
require 'msgpack'
require 'json'
require 'thread'

module SoliDB
  class Client
    MAGIC_HEADER = "solidb-drv-v1\0"
    MAX_MESSAGE_SIZE = 16 * 1024 * 1024
    DEFAULT_POOL_SIZE = 4

    # Inside a transaction the server stages these when they arrive wrapped in
    # a `transaction_command` on the connection that began it (a read-only
    # query runs on committed data; a writing one is refused there).
    STAGED_IN_TRANSACTION = %w[insert update delete query].freeze
    # These cannot be staged: refused rather than applied outside the transaction.
    REFUSED_IN_TRANSACTION = %w[bulk_insert batch begin_transaction].freeze

    attr_reader :host, :port, :database

    def initialize(host = '127.0.0.1', port = 6745, pool_size = DEFAULT_POOL_SIZE)
      @host = host
      @port = port
      @pool_size = pool_size
      @pool = []
      @pool_index = 0
      @pool_mutex = Mutex.new
      @connected = false
      @database = nil
      # The open transaction and the pooled connection it lives on: the
      # server keeps a driver transaction per connection.
      @tx = nil
    end

    def use_database(name)
      @database = name
      self
    end

    def scripts
      @scripts ||= ScriptsClient.new(self)
    end

    def triggers
      @triggers ||= TriggersClient.new(self)
    end

    def env
      @env ||= EnvClient.new(self)
    end

    def roles
      @roles ||= RolesClient.new(self)
    end

    def users
      @users ||= UsersClient.new(self)
    end

    def api_keys
      @api_keys ||= ApiKeysClient.new(self)
    end

    def cluster
      @cluster ||= ClusterClient.new(self)
    end

    def collections_ops
      @collections_ops ||= CollectionsOpsClient.new(self)
    end

    def indexes_ops
      @indexes_ops ||= IndexesOpsClient.new(self)
    end

    def geo
      @geo ||= GeoClient.new(self)
    end

    def vector
      @vector ||= VectorClient.new(self)
    end

    def ttl
      @ttl ||= TTLClient.new(self)
    end

    def columnar
      @columnar ||= ColumnarClient.new(self)
    end

    private def create_connection
      socket = TCPSocket.new(@host, @port)
      socket.setsockopt(Socket::IPPROTO_TCP, Socket::TCP_NODELAY, 1)
      socket.write(MAGIC_HEADER)
      socket
    rescue => e
      raise ConnectionError, "Failed to connect to #{@host}:#{@port} - #{e.message}"
    end

    def connect
      return if @connected && !@pool.empty?

      @pool = []
      @pool_size.times do
        @pool << create_connection
      end
      @connected = true
    end

    def close
      @pool.each { |s| s.close rescue nil }
      @pool = []
      @connected = false
    end

    private def get_next_connection
      @pool_mutex.synchronize do
        conn = @pool[@pool_index]
        @pool_index = (@pool_index + 1) % @pool.size
        conn
      end
    end

    # Public API

    def ping
      start = Time.now
      send_command("ping")
      (Time.now - start) * 1000 # returns ms
    end

    # Authentication is per connection: every pooled one must log in, or the
    # commands that land on the others are refused (3 in 4 with the default
    # pool, when only the first one was).
    def auth(database, username, password)
      auth_all("cmd" => "auth", "database" => database, "username" => username, "password" => password)
    end

    def auth_with_api_key(database, api_key)
      auth_all("cmd" => "auth", "database" => database, "username" => "", "password" => "", "api_key" => api_key)
    end

    # --- Database Operations ---

    def list_databases
      send_command("list_databases") || []
    end

    def create_database(name)
      send_command("create_database", name: name)
      nil
    end

    def delete_database(name)
      send_command("delete_database", name: name)
      nil
    end

    # --- Collection Operations ---

    def list_collections(database)
      send_command("list_collections", database: database) || []
    end

    def create_collection(database, name, type = nil)
      params = { database: database, name: name }
      params[:type] = type if type
      send_command("create_collection", params)
      nil
    end

    def delete_collection(database, name)
      send_command("delete_collection", database: database, name: name)
      nil
    end

    def collection_stats(database, name)
      send_command("collection_stats", database: database, name: name) || {}
    end

    # --- Document Operations ---

    def insert(database, collection, document, key = nil)
      params = {
        database: database,
        collection: collection,
        document: document
      }
      params[:key] = key if key
      res = send_command("insert", params)
      # Protocol returns the document (map)
      res
    end

    def get(database, collection, key)
      begin
        send_command("get", database: database, collection: collection, key: key)
      rescue ServerError => e
        # If not found, server might return error. 
        # Check if error message indicates not found or re-raise
        # For now re-raise to be consistent with other clients
        raise e
      end
    end

    def update(database, collection, key, document, merge = true)
      send_command("update", 
        database: database, 
        collection: collection, 
        key: key, 
        document: document, 
        merge: merge
      )
      nil
    end

    def delete(database, collection, key)
      send_command("delete", database: database, collection: collection, key: key)
      nil
    end
    
    def list(database, collection, limit = 50, offset = 0)
        send_command("list",
            database: database,
            collection: collection,
            limit: limit,
            offset: offset
        ) || []
    end

    # --- Query Operations ---

    def query(database, sdbql, bind_vars = {})
      send_command("query", database: database, sdbql: sdbql, bind_vars: bind_vars) || []
    end

    def explain(database, sdbql, bind_vars = {})
      send_command("explain", database: database, sdbql: sdbql, bind_vars: bind_vars) || {}
    end

    # --- Transaction Operations ---
    
    # Open a transaction on one of the pool's connections and return its id.
    #
    # Until it is committed or rolled back, insert, update and delete are
    # staged in it and applied at commit, and a read-only query runs on
    # committed data (it does not see the transaction's own writes); a query
    # that writes and a second begin_transaction are refused. get is not part
    # of the transaction: it reads committed data. One transaction at a time
    # per client.
    def begin_transaction(database, isolation_level = "read_committed")
      raise TransactionError, "a transaction is already open on this client: commit or roll it back first" if @tx
      connect unless @connected && !@pool.empty?
      socket = get_next_connection
      tx_id = send_on(socket, "cmd" => "begin_transaction", "database" => database,
                              "isolation_level" => isolation_level)
      raise ProtocolError, "begin_transaction: no transaction id in the response" unless tx_id.is_a?(String)
      @tx = { id: tx_id, socket: socket }
      tx_id
    end

    # Commit the open transaction, on the connection that began it.
    def commit_transaction(tx_id)
      end_transaction("commit_transaction", tx_id)
    end

    # Roll back the open transaction: nothing it staged is applied.
    def rollback_transaction(tx_id)
      end_transaction("rollback_transaction", tx_id)
    end

    def in_transaction?
      !@tx.nil?
    end

    # --- Index Operations ---
     def create_index(database, collection, name, fields, unique = false, sparse = false)
        send_command("create_index", 
            database: database, collection: collection, 
            name: name, fields: fields, 
            unique: unique, sparse: sparse
        )
        nil
    end

    def list_indexes(database, collection)
        send_command("list_indexes", database: database, collection: collection) || []
    end

    def delete_index(database, collection, name)
         send_command("delete_index", database: database, collection: collection, name: name)
         nil
    end


    def send_command(cmd_name, params = {})
      connect unless @connected && !@pool.empty?

      command = params.merge("cmd" => cmd_name)
      command = command.transform_keys(&:to_s)
      if @tx
        if STAGED_IN_TRANSACTION.include?(cmd_name)
          return send_on(@tx[:socket],
                         "cmd" => "transaction_command", "tx_id" => @tx[:id], "command" => command)
        end
        if REFUSED_IN_TRANSACTION.include?(cmd_name)
          raise TransactionError, "#{cmd_name} cannot run inside a transaction: commit or roll back first"
        end
      end
      send_on(get_next_connection, command)
    end

    private

    def send_on(socket, command)
      payload = MessagePack.pack(command)
      header = [payload.bytesize].pack("N")

      socket.write(header + payload)
      receive_response(socket)
    rescue Errno::EPIPE, IOError, Errno::ECONNRESET => e
      @connected = false
      raise ConnectionError, "Connection lost: #{e.message}"
    end

    def auth_all(command)
      connect unless @connected && !@pool.empty?
      @pool.each { |socket| send_on(socket, command) }
      nil
    end

    def end_transaction(cmd_name, tx_id)
      tx = @tx
      raise TransactionError, "transaction #{tx_id} is not open on this client" if tx.nil? || tx[:id] != tx_id

      # Whatever the outcome, the client has no open transaction afterwards.
      @tx = nil
      send_on(tx[:socket], "cmd" => cmd_name, "tx_id" => tx_id)
      nil
    end

    # The server's error as text: a string, or {Kind => "message"}.
    def error_message(body)
      return body if body.is_a?(String)
      return body.values.first.to_s if body.is_a?(Hash) && body.size == 1

      body.inspect
    end

    def receive_response(socket)
      header = socket.read(4)
      unless header && header.bytesize == 4
        @connected = false
        raise ConnectionError, "Server closed connection"
      end

      length = header.unpack1("N")
      raise ProtocolError, "Message too large: #{length} bytes" if length > MAX_MESSAGE_SIZE

      data = socket.read(length)
      unless data && data.bytesize == length
        @connected = false
        raise ConnectionError, "Incomplete response"
      end

      response = MessagePack.unpack(data)

      if response.is_a?(Array) && response.size >= 1 && response[0].is_a?(String)
        status = response[0]
        body = response[1]

        case status
        when "ok"
           return body
        when "error"
           raise ServerError, error_message(body)
        when "pong"
           return body
        else
           return body
        end
      end

      if response.is_a?(Hash)
         raise ServerError, error_message(response["error"]) if response["status"] == "error"
         # A begin answers with `tx_id` and no data.
         return response["tx_id"] if !response.key?("data") && response.key?("tx_id")
         return response["data"]
      end

      response
    end
  end
end
