from .client import Client
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
