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

/** A transaction call made out of turn, or a command that cannot run in one. */
export class TransactionError extends SoliDBError {
    constructor(message: string) {
        super(message);
        this.name = 'TransactionError';
    }
}

export class ProtocolError extends SoliDBError {
    constructor(message: string) {
        super(message);
        this.name = 'ProtocolError';
    }
}
