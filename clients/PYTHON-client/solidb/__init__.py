from .client import (
    Client,
    ScriptsClient,
    JobsClient,
    CronClient,
    TriggersClient,
    EnvClient,
    RolesClient,
    UsersClient,
    ApiKeysClient,
    ClusterClient,
    CollectionsClient,
    IndexesClient,
    GeoClient,
    VectorClient,
    TtlClient,
    ColumnarClient,
)
from .exceptions import SoliDBError, ConnectionError, AuthError, ServerError
from .ai import (
    AIClient,
    AIClientError,
    ContributionType,
    AgentType,
    TaskType,
    TaskStatus,
    FeedbackType,
    PatternType,
    CircuitState,
    create_worker,
)

__all__ = [
    # Wire protocol client
    "Client",
    # Sub-clients for management APIs
    "ScriptsClient",
    "JobsClient",
    "CronClient",
    "TriggersClient",
    "EnvClient",
    "RolesClient",
    "UsersClient",
    "ApiKeysClient",
    "ClusterClient",
    "CollectionsClient",
    "IndexesClient",
    "GeoClient",
    "VectorClient",
    "TtlClient",
    "ColumnarClient",
    # Exceptions
    "SoliDBError",
    "ConnectionError",
    "AuthError",
    "ServerError",
    # AI Client
    "AIClient",
    "AIClientError",
    # AI Enums
    "ContributionType",
    "AgentType",
    "TaskType",
    "TaskStatus",
    "FeedbackType",
    "PatternType",
    "CircuitState",
    # AI Helpers
    "create_worker",
]
