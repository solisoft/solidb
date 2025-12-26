export class SoliDBError extends Error {
    constructor(message: string) {
        super(message);
        this.name = 'SoliDBError';
    }
}

export class ConnectionError extends SoliDBError {
    constructor(message: string) {
        super(message);
        this.name = 'ConnectionError';
    }
}

export class ServerError extends SoliDBError {
    constructor(message: string) {
        super(message);
        this.name = 'ServerError';
    }
}

export class ProtocolError extends SoliDBError {
    constructor(message: string) {
        super(message);
        this.name = 'ProtocolError';
    }
}
