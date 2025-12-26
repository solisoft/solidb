module SoliDB
  class Error < StandardError; end
  class ConnectionError < Error; end
  class AuthError < Error; end
  class ServerError < Error; end
  class ProtocolError < Error; end
end
