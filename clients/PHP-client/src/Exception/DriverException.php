<?php

namespace SoliDB\Exception;

use RuntimeException;

class DriverException extends RuntimeException
{
    private $errorType;

    public function __construct(string $message = "", string $errorCode = "unknown_error", ?\Throwable $previous = null)
    {
        $this->errorType = $errorCode; // Assuming errorType is intended to be errorCode
        parent::__construct($message, 0, $previous);
    }

    public function getErrorType(): string
    {
        return $this->errorType;
    }
}
