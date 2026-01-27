export declare class SoliDBError extends Error {
    constructor(message: string);
}
export declare class ConnectionError extends SoliDBError {
    constructor(message: string);
}
export declare class ServerError extends SoliDBError {
    constructor(message: string);
}
export declare class ProtocolError extends SoliDBError {
    constructor(message: string);
}
