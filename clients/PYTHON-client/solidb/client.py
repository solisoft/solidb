import socket
import msgpack
import struct
import json
from .exceptions import ConnectionError, ServerError, ProtocolError

class Client:
    MAGIC_HEADER = b"solidb-drv-v1\x00"
    MAX_MESSAGE_SIZE = 16 * 1024 * 1024

    def __init__(self, host='127.0.0.1', port=6745):
        self.host = host
        self.port = port
        self.sock = None
        self.connected = False
        # Use raw=False to decode strings as UTF-8 str instead of bytes
        self.packer = msgpack.Packer(use_bin_type=True) 

    def connect(self):
        if self.connected:
            return
        try:
            self.sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            self.sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
            self.sock.connect((self.host, self.port))
            self.sock.sendall(self.MAGIC_HEADER)
            self.connected = True
        except Exception as e:
            self.connected = False
            raise ConnectionError(f"Failed to connect to {self.host}:{self.port} - {str(e)}")

    def close(self):
        if self.sock:
            try:
                self.sock.close()
            except:
                pass
        self.sock = None
        self.connected = False

    def _send_command(self, cmd_name, **kwargs):
        if not self.connected:
            self.connect()
        
        command = {"cmd": cmd_name}
        command.update(kwargs)
        
        try:
            payload = self.packer.pack(command)
            # Big-endian 4-byte length
            header = struct.pack(">I", len(payload))
            self.sock.sendall(header + payload)
            
            return self._receive_response()
        except (socket.error, BrokenPipeError) as e:
            self.connected = False
            raise ConnectionError(f"Connection lost: {str(e)}")

    def _receive_response(self):
        def recv_all(n):
            data = b''
            while len(data) < n:
                packet = self.sock.recv(n - len(data))
                if not packet:
                    return None
                data += packet
            return data

        header = recv_all(4)
        if not header:
            self.connected = False
            raise ConnectionError("Server closed connection")
        
        length = struct.unpack(">I", header)[0]
        if length > self.MAX_MESSAGE_SIZE:
             raise ProtocolError(f"Message too large: {length} bytes")

        data = recv_all(length)
        if not data:
            self.connected = False
            raise ConnectionError("Incomplete response")
            
        try:
            # raw=False decodes strings automatically
            response = msgpack.unpackb(data, raw=False)
        except Exception as e:
            raise ProtocolError(f"Failed to deserialize response: {str(e)}")

        # Handle Tuple format [status, body]
        if isinstance(response, list) and len(response) >= 1 and isinstance(response[0], str):
            status = response[0]
            body = response[1] if len(response) > 1 else None
            
            if status == "ok":
                return body
            elif status == "error":
                msg = str(body)
                if isinstance(body, dict) and len(body) == 1:
                     msg = list(body.values())[0]
                raise ServerError(msg)
            elif status == "pong":
                return body
            else:
                return body

        # Handle Map format (Legacy or Future)
        if isinstance(response, dict):
             if response.get("status") == "error":
                 raise ServerError(str(response.get("error", "Unknown error")))
             if response.get("status") == "ok":
                 return response.get("data")
             return response

        return response

    # --- Public API ---

    def ping(self):
        self._send_command("ping")
        return True

    def auth(self, database, username, password):
        self._send_command("auth", database=database, username=username, password=password)

    # Database
    def list_databases(self):
        return self._send_command("list_databases") or []

    def create_database(self, name):
        self._send_command("create_database", name=name)

    def delete_database(self, name):
        self._send_command("delete_database", name=name)

    # Collection
    def list_collections(self, database):
        return self._send_command("list_collections", database=database) or []

    def create_collection(self, database, name, type=None):
        args = {"database": database, "name": name}
        if type:
            args["type"] = type
        self._send_command("create_collection", **args)

    def delete_collection(self, database, name):
        self._send_command("delete_collection", database=database, name=name)

    def collection_stats(self, database, name):
        return self._send_command("collection_stats", database=database, name=name) or {}

    # Document
    def insert(self, database, collection, document, key=None):
        res = self._send_command("insert", database=database, collection=collection, document=document, key=key)
        # Returns the inserted document (map)
        return res

    def get(self, database, collection, key):
        return self._send_command("get", database=database, collection=collection, key=key)

    def update(self, database, collection, key, document, merge=True):
        self._send_command("update", database=database, collection=collection, key=key, document=document, merge=merge)

    def delete(self, database, collection, key):
        self._send_command("delete", database=database, collection=collection, key=key)

    def list_documents(self, database, collection, limit=50, offset=0):
        return self._send_command("list", database=database, collection=collection, limit=limit, offset=offset) or []

    # Query
    def query(self, database, sdbql, bind_vars=None):
        return self._send_command("query", database=database, sdbql=sdbql, bind_vars=bind_vars or {}) or []

    def explain(self, database, sdbql, bind_vars=None):
         return self._send_command("explain", database=database, sdbql=sdbql, bind_vars=bind_vars or {}) or {}

    # Transactions
    def begin_transaction(self, database, isolation_level="read_committed"):
        return self._send_command("begin_transaction", database=database, isolation_level=isolation_level)
    
    def commit_transaction(self, tx_id):
        self._send_command("commit_transaction", tx_id=tx_id)

    def rollback_transaction(self, tx_id):
        self._send_command("rollback_transaction", tx_id=tx_id)

    # Index
    def create_index(self, database, collection, name, fields, unique=False, sparse=False):
         self._send_command("create_index", database=database, collection=collection, 
                            name=name, fields=fields, unique=unique, sparse=sparse)

    def list_indexes(self, database, collection):
         return self._send_command("list_indexes", database=database, collection=collection) or []

    def delete_index(self, database, collection, name):
         self._send_command("delete_index", database=database, collection=collection, name=name)
