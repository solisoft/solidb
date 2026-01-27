import socket
import msgpack
import struct
import threading
import json
import urllib.request
import urllib.error
from typing import Optional, Dict, Any, List
from .exceptions import ConnectionError, ServerError, ProtocolError, AuthError


class Client:
    MAGIC_HEADER = b"solidb-drv-v1\x00"
    MAX_MESSAGE_SIZE = 16 * 1024 * 1024
    DEFAULT_POOL_SIZE = 4

    def __init__(self, host='127.0.0.1', port=6745, pool_size: int = DEFAULT_POOL_SIZE):
        self.host = host
        self.port = port
        self.pool_size = pool_size
        self._pool: List[socket.socket] = []
        self._pool_index = 0
        self._pool_lock = threading.Lock()
        self.connected = False
        self.packer = msgpack.Packer(use_bin_type=True)

        self._database: Optional[str] = None

        self._scripts: Optional['ScriptsClient'] = None
        self._jobs: Optional['JobsClient'] = None
        self._cron: Optional['CronClient'] = None
        self._triggers: Optional['TriggersClient'] = None
        self._env: Optional['EnvClient'] = None
        self._roles: Optional['RolesClient'] = None
        self._users: Optional['UsersClient'] = None
        self._api_keys: Optional['ApiKeysClient'] = None
        self._cluster: Optional['ClusterClient'] = None
        self._collections_mgmt: Optional['CollectionsClient'] = None
        self._indexes_mgmt: Optional['IndexesClient'] = None
        self._geo: Optional['GeoClient'] = None
        self._vector: Optional['VectorClient'] = None
        self._ttl: Optional['TtlClient'] = None
        self._columnar: Optional['ColumnarClient'] = None

    def _create_socket(self) -> socket.socket:
        sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        sock.connect((self.host, self.port))
        sock.sendall(self.MAGIC_HEADER)
        return sock

    def connect(self):
        if self.connected and self._pool:
            return

        self._pool = []
        for _ in range(self.pool_size):
            try:
                self._pool.append(self._create_socket())
            except Exception as e:
                for s in self._pool:
                    try:
                        s.close()
                    except:
                        pass
                self._pool = []
                raise ConnectionError(f"Failed to connect to {self.host}:{self.port} - {str(e)}")

        self.connected = True

    def close(self):
        with self._pool_lock:
            for sock in self._pool:
                try:
                    sock.close()
                except:
                    pass
            self._pool = []
            self.connected = False

    def _get_next_socket(self) -> socket.socket:
        with self._pool_lock:
            sock = self._pool[self._pool_index]
            self._pool_index = (self._pool_index + 1) % len(self._pool)
            return sock

    def _send_command(self, cmd_name, **kwargs):
        if not self.connected or not self._pool:
            self.connect()

        sock = self._get_next_socket()
        command = {"cmd": cmd_name}
        command.update(kwargs)

        try:
            payload = self.packer.pack(command)
            header = struct.pack(">I", len(payload))
            sock.sendall(header + payload)

            return self._receive_response(sock)
        except (socket.error, BrokenPipeError, OSError) as e:
            self.connected = False
            raise ConnectionError(f"Connection lost: {str(e)}")

    def _receive_response(self, sock: socket.socket):
        def recv_all(n):
            data = b''
            while len(data) < n:
                packet = sock.recv(n - len(data))
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
            response = msgpack.unpackb(data, raw=False)
        except Exception as e:
            raise ProtocolError(f"Failed to deserialize response: {str(e)}")

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

    def auth_with_api_key(self, database, api_key):
        self._send_command("auth", database=database, username="", password="", api_key=api_key)

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

    # --- Database Context for Management APIs ---

    def use_database(self, name: str) -> 'Client':
        """Set the default database for management operations."""
        self._database = name
        return self

    @property
    def database(self) -> str:
        """Get the current database context. Raises if not set."""
        if self._database is None:
            raise ValueError("No database selected. Call use_database() first.")
        return self._database

    # --- HTTP Request Method for Management APIs ---

    def _http_request(self, method: str, path: str, body: Any = None, params: Dict = None) -> Any:
        """
        Make an HTTP request to the SoliDB REST API.
        Used by sub-clients for management operations.
        """
        url = f"http://{self.host}:{self.http_port}{path}"
        if params:
            query = "&".join(f"{k}={v}" for k, v in params.items() if v is not None)
            if query:
                url = f"{url}?{query}"

        headers = {"Content-Type": "application/json"}
        if self._token:
            headers["Authorization"] = f"Bearer {self._token}"

        data = None
        if body is not None:
            data = json.dumps(body).encode('utf-8')

        req = urllib.request.Request(url, data=data, headers=headers, method=method)

        try:
            with urllib.request.urlopen(req, timeout=30) as response:
                if response.status == 204:
                    return None
                return json.loads(response.read().decode('utf-8'))
        except urllib.error.HTTPError as e:
            error_body = e.read().decode('utf-8') if e.fp else str(e)
            try:
                error_data = json.loads(error_body)
                error_msg = error_data.get('error', error_body)
            except:
                error_msg = error_body

            if e.code == 401:
                raise AuthError(f"Authentication failed: {error_msg}")
            elif e.code == 403:
                raise AuthError(f"Access denied: {error_msg}")
            elif e.code == 404:
                raise ServerError(f"Not found: {error_msg}")
            else:
                raise ServerError(f"HTTP {e.code}: {error_msg}")
        except urllib.error.URLError as e:
            raise ConnectionError(f"Failed to connect: {e.reason}")

    def _http_get(self, path: str, params: Dict = None) -> Any:
        return self._http_request("GET", path, params=params)

    def _http_post(self, path: str, body: Any = None) -> Any:
        return self._http_request("POST", path, body=body)

    def _http_put(self, path: str, body: Any = None) -> Any:
        return self._http_request("PUT", path, body=body)

    def _http_delete(self, path: str) -> Any:
        return self._http_request("DELETE", path)

    # --- Login for HTTP API ---

    def login(self, database: str, username: str, password: str) -> str:
        """
        Login via HTTP API and get a JWT token.
        Sets the token for subsequent HTTP requests.
        """
        result = self._http_post("/auth/login", {
            "database": database,
            "username": username,
            "password": password
        })
        self._token = result.get("token")
        self._database = database
        return self._token

    # --- Sub-Client Properties ---

    @property
    def scripts(self) -> 'ScriptsClient':
        """Access scripts management API."""
        if self._scripts is None:
            self._scripts = ScriptsClient(self)
        return self._scripts

    @property
    def jobs(self) -> 'JobsClient':
        """Access jobs/queue management API."""
        if self._jobs is None:
            self._jobs = JobsClient(self)
        return self._jobs

    @property
    def cron(self) -> 'CronClient':
        """Access cron jobs management API."""
        if self._cron is None:
            self._cron = CronClient(self)
        return self._cron

    @property
    def triggers(self) -> 'TriggersClient':
        """Access triggers management API."""
        if self._triggers is None:
            self._triggers = TriggersClient(self)
        return self._triggers

    @property
    def env(self) -> 'EnvClient':
        """Access environment variables management API."""
        if self._env is None:
            self._env = EnvClient(self)
        return self._env

    @property
    def roles(self) -> 'RolesClient':
        """Access roles management API."""
        if self._roles is None:
            self._roles = RolesClient(self)
        return self._roles

    @property
    def users(self) -> 'UsersClient':
        """Access users management API."""
        if self._users is None:
            self._users = UsersClient(self)
        return self._users

    @property
    def api_keys(self) -> 'ApiKeysClient':
        """Access API keys management API."""
        if self._api_keys is None:
            self._api_keys = ApiKeysClient(self)
        return self._api_keys

    @property
    def cluster(self) -> 'ClusterClient':
        """Access cluster management API."""
        if self._cluster is None:
            self._cluster = ClusterClient(self)
        return self._cluster

    @property
    def collections_mgmt(self) -> 'CollectionsClient':
        """Access advanced collection management API (truncate, compact, schema, etc.)."""
        if self._collections_mgmt is None:
            self._collections_mgmt = CollectionsClient(self)
        return self._collections_mgmt

    @property
    def indexes_mgmt(self) -> 'IndexesClient':
        """Access advanced index management API (rebuild, hybrid search)."""
        if self._indexes_mgmt is None:
            self._indexes_mgmt = IndexesClient(self)
        return self._indexes_mgmt

    @property
    def geo(self) -> 'GeoClient':
        """Access geo index management API."""
        if self._geo is None:
            self._geo = GeoClient(self)
        return self._geo

    @property
    def vector(self) -> 'VectorClient':
        """Access vector index management API."""
        if self._vector is None:
            self._vector = VectorClient(self)
        return self._vector

    @property
    def ttl(self) -> 'TtlClient':
        """Access TTL index management API."""
        if self._ttl is None:
            self._ttl = TtlClient(self)
        return self._ttl

    @property
    def columnar(self) -> 'ColumnarClient':
        """Access columnar storage management API."""
        if self._columnar is None:
            self._columnar = ColumnarClient(self)
        return self._columnar


# =============================================================================
# SUB-CLIENTS FOR MANAGEMENT APIs
# =============================================================================


class ScriptsClient:
    """Client for Lua scripts management API."""

    def __init__(self, client: Client):
        self._client = client

    def create(self, name: str, path: str, methods: List[str], code: str,
               description: str = None, collection: str = None) -> Dict:
        """Create a new Lua script."""
        payload = {
            "name": name,
            "path": path,
            "methods": methods,
            "code": code
        }
        if description:
            payload["description"] = description
        if collection:
            payload["collection"] = collection
        return self._client._http_post(f"/_api/database/{self._client.database}/scripts", payload)

    def list(self) -> List[Dict]:
        """List all scripts in the database."""
        result = self._client._http_get(f"/_api/database/{self._client.database}/scripts")
        return result.get("scripts", [])

    def get(self, script_id: str) -> Dict:
        """Get a script by ID."""
        return self._client._http_get(f"/_api/database/{self._client.database}/scripts/{script_id}")

    def update(self, script_id: str, **kwargs) -> Dict:
        """Update a script. Pass any fields to update (name, path, methods, code, description)."""
        return self._client._http_put(f"/_api/database/{self._client.database}/scripts/{script_id}", kwargs)

    def delete(self, script_id: str) -> None:
        """Delete a script."""
        self._client._http_delete(f"/_api/database/{self._client.database}/scripts/{script_id}")

    def get_stats(self) -> Dict:
        """Get script execution statistics."""
        return self._client._http_get("/_api/scripts/stats")


class JobsClient:
    """Client for queue/jobs management API."""

    def __init__(self, client: Client):
        self._client = client

    def list_queues(self) -> List[Dict]:
        """List all queues."""
        result = self._client._http_get(f"/_api/database/{self._client.database}/queues")
        return result.get("queues", [])

    def list_jobs(self, queue_name: str, status: str = None, limit: int = 50, offset: int = 0) -> List[Dict]:
        """List jobs in a queue."""
        params = {"limit": limit, "offset": offset}
        if status:
            params["status"] = status
        result = self._client._http_get(f"/_api/database/{self._client.database}/queues/{queue_name}/jobs", params)
        return result.get("jobs", [])

    def enqueue(self, queue_name: str, script_path: str, params: Dict = None,
                priority: int = 0, run_at: str = None, max_retries: int = 3) -> Dict:
        """Enqueue a new job."""
        payload = {
            "script_path": script_path,
            "params": params or {},
            "priority": priority,
            "max_retries": max_retries
        }
        if run_at:
            payload["run_at"] = run_at
        return self._client._http_post(f"/_api/database/{self._client.database}/queues/{queue_name}/enqueue", payload)

    def cancel(self, job_id: str) -> None:
        """Cancel a job."""
        self._client._http_delete(f"/_api/database/{self._client.database}/queues/jobs/{job_id}")


class CronClient:
    """Client for cron jobs management API."""

    def __init__(self, client: Client):
        self._client = client

    def list(self) -> List[Dict]:
        """List all cron jobs."""
        result = self._client._http_get(f"/_api/database/{self._client.database}/cron")
        return result.get("cron_jobs", [])

    def create(self, name: str, cron_expr: str, script_path: str, params: Dict = None,
               queue: str = "default", priority: int = 0, max_retries: int = 3) -> Dict:
        """Create a new cron job."""
        payload = {
            "name": name,
            "cron_expression": cron_expr,
            "script_path": script_path,
            "params": params or {},
            "queue": queue,
            "priority": priority,
            "max_retries": max_retries
        }
        return self._client._http_post(f"/_api/database/{self._client.database}/cron", payload)

    def update(self, cron_id: str, **kwargs) -> Dict:
        """Update a cron job."""
        return self._client._http_put(f"/_api/database/{self._client.database}/cron/{cron_id}", kwargs)

    def delete(self, cron_id: str) -> None:
        """Delete a cron job."""
        self._client._http_delete(f"/_api/database/{self._client.database}/cron/{cron_id}")


class TriggersClient:
    """Client for triggers management API."""

    def __init__(self, client: Client):
        self._client = client

    def list(self) -> List[Dict]:
        """List all triggers in the database."""
        result = self._client._http_get(f"/_api/database/{self._client.database}/triggers")
        return result.get("triggers", [])

    def list_by_collection(self, collection: str) -> List[Dict]:
        """List triggers for a specific collection."""
        result = self._client._http_get(f"/_api/database/{self._client.database}/collections/{collection}/triggers")
        return result.get("triggers", [])

    def create(self, name: str, collection: str, events: List[str], script_path: str,
               filter_expr: str = None, queue: str = "default", priority: int = 0,
               max_retries: int = 3, enabled: bool = True) -> Dict:
        """Create a new trigger."""
        payload = {
            "name": name,
            "collection": collection,
            "events": events,  # ["insert", "update", "delete"]
            "script_path": script_path,
            "queue": queue,
            "priority": priority,
            "max_retries": max_retries,
            "enabled": enabled
        }
        if filter_expr:
            payload["filter"] = filter_expr
        return self._client._http_post(f"/_api/database/{self._client.database}/triggers", payload)

    def get(self, trigger_id: str) -> Dict:
        """Get a trigger by ID."""
        return self._client._http_get(f"/_api/database/{self._client.database}/triggers/{trigger_id}")

    def update(self, trigger_id: str, **kwargs) -> Dict:
        """Update a trigger."""
        return self._client._http_put(f"/_api/database/{self._client.database}/triggers/{trigger_id}", kwargs)

    def delete(self, trigger_id: str) -> None:
        """Delete a trigger."""
        self._client._http_delete(f"/_api/database/{self._client.database}/triggers/{trigger_id}")

    def toggle(self, trigger_id: str) -> Dict:
        """Toggle a trigger's enabled state."""
        return self._client._http_post(f"/_api/database/{self._client.database}/triggers/{trigger_id}/toggle")


class EnvClient:
    """Client for environment variables management API."""

    def __init__(self, client: Client):
        self._client = client

    def list(self) -> Dict[str, str]:
        """List all environment variables."""
        result = self._client._http_get(f"/_api/database/{self._client.database}/env")
        return result.get("variables", {})

    def set(self, key: str, value: str) -> None:
        """Set an environment variable."""
        self._client._http_put(f"/_api/database/{self._client.database}/env/{key}", {"value": value})

    def delete(self, key: str) -> None:
        """Delete an environment variable."""
        self._client._http_delete(f"/_api/database/{self._client.database}/env/{key}")


class RolesClient:
    """Client for roles management API."""

    def __init__(self, client: Client):
        self._client = client

    def list(self) -> List[Dict]:
        """List all roles."""
        result = self._client._http_get("/_api/auth/roles")
        return result.get("roles", [])

    def create(self, name: str, permissions: List[Dict]) -> Dict:
        """
        Create a new role.

        Args:
            name: Role name
            permissions: List of permission objects with 'action', 'scope', and optional 'database'
                         e.g., [{"action": "read", "scope": "database", "database": "mydb"}]
        """
        return self._client._http_post("/_api/auth/roles", {"name": name, "permissions": permissions})

    def get(self, name: str) -> Dict:
        """Get a role by name."""
        return self._client._http_get(f"/_api/auth/roles/{name}")

    def update(self, name: str, permissions: List[Dict]) -> Dict:
        """Update a role's permissions."""
        return self._client._http_put(f"/_api/auth/roles/{name}", {"permissions": permissions})

    def delete(self, name: str) -> None:
        """Delete a role."""
        self._client._http_delete(f"/_api/auth/roles/{name}")


class UsersClient:
    """Client for users management API."""

    def __init__(self, client: Client):
        self._client = client

    def list(self) -> List[Dict]:
        """List all users."""
        result = self._client._http_get("/_api/auth/users")
        return result.get("users", [])

    def create(self, username: str, password: str, roles: List[str] = None) -> Dict:
        """Create a new user."""
        payload = {"username": username, "password": password}
        if roles:
            payload["roles"] = roles
        return self._client._http_post("/_api/auth/users", payload)

    def delete(self, username: str) -> None:
        """Delete a user."""
        self._client._http_delete(f"/_api/auth/users/{username}")

    def get_roles(self, username: str) -> List[Dict]:
        """Get roles assigned to a user."""
        result = self._client._http_get(f"/_api/auth/users/{username}/roles")
        return result.get("roles", [])

    def assign_role(self, username: str, role: str, database: str = None) -> Dict:
        """Assign a role to a user."""
        payload = {"role": role}
        if database:
            payload["database"] = database
        return self._client._http_post(f"/_api/auth/users/{username}/roles", payload)

    def revoke_role(self, username: str, role: str) -> None:
        """Revoke a role from a user."""
        self._client._http_delete(f"/_api/auth/users/{username}/roles/{role}")

    def me(self) -> Dict:
        """Get current authenticated user info."""
        return self._client._http_get("/_api/auth/me")

    def my_permissions(self) -> Dict:
        """Get current user's permissions."""
        return self._client._http_get("/_api/auth/me/permissions")


class ApiKeysClient:
    """Client for API keys management API."""

    def __init__(self, client: Client):
        self._client = client

    def list(self) -> List[Dict]:
        """List all API keys."""
        result = self._client._http_get("/_api/auth/api-keys")
        return result.get("api_keys", [])

    def create(self, name: str, permissions: List[Dict] = None, expires_at: str = None) -> Dict:
        """Create a new API key."""
        payload = {"name": name}
        if permissions:
            payload["permissions"] = permissions
        if expires_at:
            payload["expires_at"] = expires_at
        return self._client._http_post("/_api/auth/api-keys", payload)

    def delete(self, key_id: str) -> None:
        """Delete an API key."""
        self._client._http_delete(f"/_api/auth/api-keys/{key_id}")


class ClusterClient:
    """Client for cluster management API."""

    def __init__(self, client: Client):
        self._client = client

    def status(self) -> Dict:
        """Get cluster status."""
        return self._client._http_get("/_api/cluster/status")

    def info(self) -> Dict:
        """Get cluster info."""
        return self._client._http_get("/_api/cluster/info")

    def remove_node(self, node_id: str) -> Dict:
        """Remove a node from the cluster."""
        return self._client._http_post("/_api/cluster/remove-node", {"node_id": node_id})

    def rebalance(self) -> Dict:
        """Trigger cluster rebalancing."""
        return self._client._http_post("/_api/cluster/rebalance")

    def cleanup(self) -> Dict:
        """Trigger cluster cleanup."""
        return self._client._http_post("/_api/cluster/cleanup")

    def reshard(self, num_shards: int = None) -> Dict:
        """Trigger cluster resharding."""
        payload = {}
        if num_shards:
            payload["num_shards"] = num_shards
        return self._client._http_post("/_api/cluster/reshard", payload)


class CollectionsClient:
    """Client for advanced collection management API."""

    def __init__(self, client: Client):
        self._client = client

    def truncate(self, collection: str) -> Dict:
        """Truncate (empty) a collection."""
        return self._client._http_put(f"/_api/database/{self._client.database}/collection/{collection}/truncate")

    def compact(self, collection: str) -> Dict:
        """Compact a collection's storage."""
        return self._client._http_put(f"/_api/database/{self._client.database}/collection/{collection}/compact")

    def prune(self, collection: str) -> Dict:
        """Prune old/deleted documents from a collection."""
        return self._client._http_post(f"/_api/database/{self._client.database}/collection/{collection}/prune")

    def recount(self, collection: str) -> Dict:
        """Recalculate document count for a collection."""
        return self._client._http_put(f"/_api/database/{self._client.database}/collection/{collection}/recount")

    def repair(self, collection: str) -> Dict:
        """Repair a collection."""
        return self._client._http_post(f"/_api/database/{self._client.database}/collection/{collection}/repair")

    def stats(self, collection: str) -> Dict:
        """Get collection statistics."""
        return self._client._http_get(f"/_api/database/{self._client.database}/collection/{collection}/stats")

    def sharding(self, collection: str) -> Dict:
        """Get collection sharding details."""
        return self._client._http_get(f"/_api/database/{self._client.database}/collection/{collection}/sharding")

    def export_data(self, collection: str) -> List[Dict]:
        """Export collection data."""
        return self._client._http_get(f"/_api/database/{self._client.database}/collection/{collection}/export")

    def import_data(self, collection: str, documents: List[Dict]) -> Dict:
        """Import documents into a collection."""
        return self._client._http_post(f"/_api/database/{self._client.database}/collection/{collection}/import", {"documents": documents})

    def set_schema(self, collection: str, schema: Dict) -> Dict:
        """Set JSON schema for a collection."""
        return self._client._http_post(f"/_api/database/{self._client.database}/collection/{collection}/schema", schema)

    def get_schema(self, collection: str) -> Dict:
        """Get JSON schema for a collection."""
        return self._client._http_get(f"/_api/database/{self._client.database}/collection/{collection}/schema")

    def delete_schema(self, collection: str) -> None:
        """Delete JSON schema for a collection."""
        self._client._http_delete(f"/_api/database/{self._client.database}/collection/{collection}/schema")


class IndexesClient:
    """Client for advanced index management API."""

    def __init__(self, client: Client):
        self._client = client

    def rebuild(self, collection: str) -> Dict:
        """Rebuild all indexes for a collection."""
        return self._client._http_put(f"/_api/database/{self._client.database}/index/{collection}/rebuild")

    def hybrid_search(self, collection: str, query: str, vector: List[float] = None,
                      vector_field: str = None, limit: int = 10, alpha: float = 0.5) -> List[Dict]:
        """Perform hybrid search combining text and vector search."""
        payload = {
            "query": query,
            "limit": limit,
            "alpha": alpha
        }
        if vector:
            payload["vector"] = vector
        if vector_field:
            payload["vector_field"] = vector_field
        result = self._client._http_post(f"/_api/database/{self._client.database}/hybrid/{collection}/search", payload)
        return result.get("results", [])


class GeoClient:
    """Client for geo index management API."""

    def __init__(self, client: Client):
        self._client = client

    def create_index(self, collection: str, name: str, field: str) -> Dict:
        """Create a geo index."""
        return self._client._http_post(f"/_api/database/{self._client.database}/geo/{collection}", {
            "name": name,
            "field": field
        })

    def list_indexes(self, collection: str) -> List[Dict]:
        """List geo indexes for a collection."""
        result = self._client._http_get(f"/_api/database/{self._client.database}/geo/{collection}")
        return result.get("indexes", [])

    def delete_index(self, collection: str, name: str) -> None:
        """Delete a geo index."""
        self._client._http_delete(f"/_api/database/{self._client.database}/geo/{collection}/{name}")

    def near(self, collection: str, field: str, latitude: float, longitude: float,
             radius: float = None, limit: int = 100) -> List[Dict]:
        """Find documents near a point."""
        payload = {
            "latitude": latitude,
            "longitude": longitude,
            "limit": limit
        }
        if radius:
            payload["radius"] = radius
        result = self._client._http_post(f"/_api/database/{self._client.database}/geo/{collection}/{field}/near", payload)
        return result.get("results", [])

    def within(self, collection: str, field: str, polygon: List[List[float]]) -> List[Dict]:
        """Find documents within a polygon."""
        result = self._client._http_post(f"/_api/database/{self._client.database}/geo/{collection}/{field}/within", {
            "polygon": polygon
        })
        return result.get("results", [])


class VectorClient:
    """Client for vector index management API."""

    def __init__(self, client: Client):
        self._client = client

    def create_index(self, collection: str, name: str, field: str, dimensions: int,
                     metric: str = "cosine", ef_construction: int = 200, m: int = 16) -> Dict:
        """Create a vector index."""
        return self._client._http_post(f"/_api/database/{self._client.database}/vector/{collection}", {
            "name": name,
            "field": field,
            "dimensions": dimensions,
            "metric": metric,
            "ef_construction": ef_construction,
            "m": m
        })

    def list_indexes(self, collection: str) -> List[Dict]:
        """List vector indexes for a collection."""
        result = self._client._http_get(f"/_api/database/{self._client.database}/vector/{collection}")
        return result.get("indexes", [])

    def delete_index(self, collection: str, name: str) -> None:
        """Delete a vector index."""
        self._client._http_delete(f"/_api/database/{self._client.database}/vector/{collection}/{name}")

    def search(self, collection: str, index_name: str, vector: List[float],
               limit: int = 10, ef_search: int = None, filter_expr: str = None) -> List[Dict]:
        """Search for similar vectors."""
        payload = {
            "vector": vector,
            "limit": limit
        }
        if ef_search:
            payload["ef_search"] = ef_search
        if filter_expr:
            payload["filter"] = filter_expr
        result = self._client._http_post(f"/_api/database/{self._client.database}/vector/{collection}/{index_name}/search", payload)
        return result.get("results", [])

    def quantize(self, collection: str, index_name: str) -> Dict:
        """Quantize a vector index for reduced memory usage."""
        return self._client._http_post(f"/_api/database/{self._client.database}/vector/{collection}/{index_name}/quantize")

    def dequantize(self, collection: str, index_name: str) -> Dict:
        """Dequantize a vector index."""
        return self._client._http_post(f"/_api/database/{self._client.database}/vector/{collection}/{index_name}/dequantize")


class TtlClient:
    """Client for TTL index management API."""

    def __init__(self, client: Client):
        self._client = client

    def create_index(self, collection: str, name: str, field: str, expire_after_seconds: int) -> Dict:
        """Create a TTL index."""
        return self._client._http_post(f"/_api/database/{self._client.database}/ttl/{collection}", {
            "name": name,
            "field": field,
            "expire_after_seconds": expire_after_seconds
        })

    def list_indexes(self, collection: str) -> List[Dict]:
        """List TTL indexes for a collection."""
        result = self._client._http_get(f"/_api/database/{self._client.database}/ttl/{collection}")
        return result.get("indexes", [])

    def delete_index(self, collection: str, name: str) -> None:
        """Delete a TTL index."""
        self._client._http_delete(f"/_api/database/{self._client.database}/ttl/{collection}/{name}")


class ColumnarClient:
    """Client for columnar storage management API."""

    def __init__(self, client: Client):
        self._client = client

    def create(self, name: str, columns: List[Dict]) -> Dict:
        """Create a columnar collection."""
        return self._client._http_post(f"/_api/database/{self._client.database}/columnar", {
            "name": name,
            "columns": columns
        })

    def list(self) -> List[Dict]:
        """List columnar collections."""
        result = self._client._http_get(f"/_api/database/{self._client.database}/columnar")
        return result.get("collections", [])

    def get(self, collection: str) -> Dict:
        """Get columnar collection details."""
        return self._client._http_get(f"/_api/database/{self._client.database}/columnar/{collection}")

    def delete(self, collection: str) -> None:
        """Delete a columnar collection."""
        self._client._http_delete(f"/_api/database/{self._client.database}/columnar/{collection}")

    def insert(self, collection: str, rows: List[Dict]) -> Dict:
        """Insert rows into a columnar collection."""
        return self._client._http_post(f"/_api/database/{self._client.database}/columnar/{collection}/insert", {
            "rows": rows
        })

    def aggregate(self, collection: str, aggregations: List[Dict], group_by: List[str] = None,
                  filter_expr: str = None) -> List[Dict]:
        """Run aggregations on a columnar collection."""
        payload = {"aggregations": aggregations}
        if group_by:
            payload["group_by"] = group_by
        if filter_expr:
            payload["filter"] = filter_expr
        result = self._client._http_post(f"/_api/database/{self._client.database}/columnar/{collection}/aggregate", payload)
        return result.get("results", [])

    def query(self, collection: str, columns: List[str] = None, filter_expr: str = None,
              order_by: str = None, limit: int = None) -> List[Dict]:
        """Query a columnar collection."""
        payload = {}
        if columns:
            payload["columns"] = columns
        if filter_expr:
            payload["filter"] = filter_expr
        if order_by:
            payload["order_by"] = order_by
        if limit:
            payload["limit"] = limit
        result = self._client._http_post(f"/_api/database/{self._client.database}/columnar/{collection}/query", payload)
        return result.get("results", [])

    def create_index(self, collection: str, column: str) -> Dict:
        """Create an index on a columnar column."""
        return self._client._http_post(f"/_api/database/{self._client.database}/columnar/{collection}/index", {
            "column": column
        })

    def list_indexes(self, collection: str) -> List[Dict]:
        """List indexes on a columnar collection."""
        result = self._client._http_get(f"/_api/database/{self._client.database}/columnar/{collection}/indexes")
        return result.get("indexes", [])

    def delete_index(self, collection: str, column: str) -> None:
        """Delete an index from a columnar collection."""
        self._client._http_delete(f"/_api/database/{self._client.database}/columnar/{collection}/index/{column}")
