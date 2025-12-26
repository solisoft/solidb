require 'socket'
require 'msgpack'
require 'json'

module SoliDB
  class Client
    MAGIC_HEADER = "solidb-drv-v1\0"
    MAX_MESSAGE_SIZE = 16 * 1024 * 1024

    attr_reader :host, :port

    def initialize(host = '127.0.0.1', port = 6745)
      @host = host
      @port = port
      @socket = nil
      @connected = false
    end

    def connect
      return if @connected
      
      begin
        @socket = TCPSocket.new(@host, @port)
        @socket.setsockopt(Socket::IPPROTO_TCP, Socket::TCP_NODELAY, 1)
        
        # Set timeouts if possible (Ruby socket timeouts are tricky without IO.select context or timeout lib)
        # For simplicity, blocking for now.
        
        @socket.write(MAGIC_HEADER)
        @connected = true
      rescue => e
        raise ConnectionError, "Failed to connect to #{@host}:#{@port} - #{e.message}"
      end
    end

    def close
      @socket&.close
      @socket = nil
      @connected = false
    end

    # Public API

    def ping
      start = Time.now
      send_command("ping")
      (Time.now - start) * 1000 # returns ms
    end

    def auth(database, username, password)
      send_command("auth", database: database, username: username, password: password)
      nil
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
    
    def begin_transaction(database, isolation_level = "read_committed")
       res = send_command("begin_transaction", database: database, isolation_level: isolation_level)
       # The response IS the tx_id string? Or contained in body?
       # Protocol: Response::ok_tx(tx_id). 
       # Structure: ["ok", {tx_id: "..."}] ?? No, definition was Ok { ... tx_id: Some(...) }.
       # If serialized as tuple, maybe ["ok", [null, null, tx_id]]?
       # Or if flattened?
       # We'll see during tests. Assuming it returns tx_id directly or we need to parse.
       
       # Wait, previously we saw `["ok", {_key: ...}]` for insert.
       # That was `Ok { data: Some(val) }`.
       # `ok_tx` is `Ok { tx_id: Some(id) }`.
       # If rmp_serde flattens structs...
       # The map `{"_key":...}` WAS the value.
       
       # If tx_id is separate field in Ok struct.
       # It might be returned as a map `{"tx_id": "..."}` or simply ignored if standard parsing logic assumes data.
       # I need to return the WHOLE body if it's a map.
       # If it return tx_id, caller expects string.
       
       # If result is Hash, look for tx_id?
       if res.is_a?(Hash) && res["tx_id"]
         return res["tx_id"]
       end
       
       # If implementation changed to return tx_id as data?
       # No, protocol helper `ok_tx` sets `tx_id` field, `data` is None.
       
       res
    end

    def commit_transaction(tx_id)
      send_command("commit_transaction", tx_id: tx_id)
      nil
    end

    def rollback_transaction(tx_id)
      send_command("rollback_transaction", tx_id: tx_id)
      nil
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


    private

    def send_command(cmd_name, params = {})
      connect unless @connected
      
      command = params.merge("cmd" => cmd_name)
      
      # Transform keys to strings for serialization if needed (MessagePack handles symbols usually, but protocol expects strings?)
      # Protocol: struct fields. `rmp_serde` usually matches exact names.
      # But `Command` enum variants are snake_case tagged.
      # The fields inside variants.
      
      # Ruby `MessagePack.pack({symbol: val})` packs keys as strings usually? No, depends on config.
      # We should force keys to be strings to be safe match against Rust structs?
      # Rust `rmp_serde` deserialization usually works with strings.
      
      # Let's ensure command map has string keys.
      command = command.transform_keys(&:to_s)
      
      payload = MessagePack.pack(command)
      header = [payload.bytesize].pack("N")
      
      @socket.write(header + payload)
      receive_response
    rescue Errno::EPIPE, IOError, Errno::ECONNRESET => e
      @connected = false
      raise ConnectionError, "Connection lost: #{e.message}"
    end

    def receive_response
      header = @socket.read(4)
      unless header && header.bytesize == 4
        @connected = false
        raise ConnectionError, "Server closed connection"
      end
      
      length = header.unpack1("N")
      raise ProtocolError, "Message too large: #{length} bytes" if length > MAX_MESSAGE_SIZE
      
      data = @socket.read(length)
      unless data && data.bytesize == length
        @connected = false
        raise ConnectionError, "Incomplete response" 
      end
      
      response = MessagePack.unpack(data)
      
      # Handle Tuple format [status, body]
      # SoliDB Rust driver sends [status_string, body_value]
      if response.is_a?(Array) && response.size >= 1 && response[0].is_a?(String)
        status = response[0]
        body = response[1]
        
        case status
        when "ok"
           return body
        when "error"
           # body is likely {"ErrorType" => "Msg"} e.g., {"DatabaseError" => "..."}
           msg = body.inspect
           if body.is_a?(Hash) && body.size == 1
             msg = body.values.first
           elsif body.is_a?(String)
             msg = body
           end
           raise ServerError, msg
        when "pong"
           return body
        else
           # Unknown status, might be success?
           return body
        end
      end
      
      # Handle Map format (unlikely based on PHP experience but possible if changed)
      if response.is_a?(Hash)
         if response["status"] == "error"
            raise ServerError, response["error"].inspect
         end
         return response["data"]
      end
      
      response
    end
  end
end
