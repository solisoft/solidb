"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.ProtocolError = exports.ServerError = exports.ConnectionError = exports.SoliDBError = void 0;
class SoliDBError extends Error {
    constructor(message) {
        super(message);
        this.name = 'SoliDBError';
    }
}
exports.SoliDBError = SoliDBError;
class ConnectionError extends SoliDBError {
    constructor(message) {
        super(message);
        this.name = 'ConnectionError';
    }
}
exports.ConnectionError = ConnectionError;
class ServerError extends SoliDBError {
    constructor(message) {
        super(message);
        this.name = 'ServerError';
    }
}
exports.ServerError = ServerError;
class ProtocolError extends SoliDBError {
    constructor(message) {
        super(message);
        this.name = 'ProtocolError';
    }
}
exports.ProtocolError = ProtocolError;
