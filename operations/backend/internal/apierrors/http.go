package apierrors

import (
	"encoding/json"
	"net/http"
	"strconv"
)

const NumberHeader = "X-Aster-Error-Number"

func NumberForCode(code string) int64 {
	if number, exists := codeNumbers[code]; exists {
		return number
	}
	return codeNumbers["INTERNAL_ERROR"]
}

func Write(response http.ResponseWriter, status int, code, message string) int64 {
	number, registered := codeNumbers[code]
	if !registered {
		code = "INTERNAL_ERROR"
		message = "服务发生内部错误"
		number = codeNumbers[code]
	}
	response.Header().Set("Content-Type", "application/json; charset=utf-8")
	response.Header().Set("Cache-Control", "no-store")
	response.Header().Set(NumberHeader, strconv.FormatInt(number, 10))
	response.WriteHeader(status)
	_ = json.NewEncoder(response).Encode(map[string]any{"error": map[string]any{
		"code": code, "message": message, "number": number,
	}})
	return number
}
