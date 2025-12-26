package solidb

import "fmt"

type SoliDBError struct {
	Message string
}

func (e *SoliDBError) Error() string {
	return fmt.Sprintf("SoliDB error: %s", e.Message)
}

type ConnectionError struct {
	SoliDBError
}

type AuthError struct {
	SoliDBError
}

type ServerError struct {
	SoliDBError
}

type ProtocolError struct {
	SoliDBError
}
