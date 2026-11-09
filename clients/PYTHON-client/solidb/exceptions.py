class SoliDBError(Exception):
    """Base exception for SoliDB errors"""
    pass

class ConnectionError(SoliDBError):
    """Raised when connection fails"""
    pass

class AuthError(SoliDBError):
    """Raised when authentication fails"""
    pass

class ServerError(SoliDBError):
    """Raised when server returns an error"""
    pass

class TransactionError(SoliDBError):
    """Raised for a transaction call made out of turn, or a command that cannot run in one"""
    pass

class ProtocolError(SoliDBError):
    """Raised when protocol violation occurs"""
    pass
