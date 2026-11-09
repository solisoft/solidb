module SoliDB
  class Error < StandardError; end
  class ConnectionError < Error; end
  class AuthError < Error; end
  class ServerError < Error; end
  class ProtocolError < Error; end
  # A transaction call made out of turn, or a command that cannot run in one.
  class TransactionError < Error; end
end
