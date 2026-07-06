package main

import (
	"encoding/json"
	"log"
	"net/http"
)

const (
	port     = "18080"
	token    = "dev-token"
	pdfURL   = "https://raw.githubusercontent.com/vergil-lai/print-bridge-jssdk/main/examples/assets/printbridge-a4-sample.pdf"
	imageURL = "https://raw.githubusercontent.com/vergil-lai/print-bridge-jssdk/main/examples/assets/printbridge-a4-sample.jpg"
)

type printBatchTask struct {
	Type      string     `json:"type"`
	RequestID string     `json:"request_id"`
	BatchID   string     `json:"batch_id"`
	Jobs      []printJob `json:"jobs"`
}

type printJob struct {
	JobID   string `json:"job_id"`
	Format  string `json:"format"`
	FileURL string `json:"file_url"`
	Copies  int    `json:"copies"`
}

func main() {
	server := &remoteTaskServer{}

	http.HandleFunc("/print-task", server.handlePrintTask)

	address := "127.0.0.1:" + port
	log.Printf("Remote task example listening on http://%s/print-task", address)
	log.Printf("Bearer token: %s", token)
	log.Fatal(http.ListenAndServe(address, nil))
}

type remoteTaskServer struct{}

func (server *remoteTaskServer) handlePrintTask(writer http.ResponseWriter, request *http.Request) {
	if request.Header.Get("Authorization") != "Bearer "+token {
		writeJSON(writer, http.StatusUnauthorized, map[string]string{"error": "unauthorized"})
		return
	}

	if request.Header.Get("X-PrintBridge-Test") == "true" {
		handleConnectionTest(writer, request)
		return
	}

	switch request.Method {
	case http.MethodGet:
		server.handleFetchTask(writer)
	case http.MethodPost:
		server.handleStatusReport(writer, request)
	default:
		writeJSON(writer, http.StatusMethodNotAllowed, map[string]string{"error": "method_not_allowed"})
	}
}

func (server *remoteTaskServer) handleFetchTask(writer http.ResponseWriter) {
	writeJSON(writer, http.StatusOK, printBatchTask{
		Type:      "print_batch",
		RequestID: "REQ-GO-BATCH",
		BatchID:   "BATCH-GO-SAMPLE",
		Jobs: []printJob{
			{
				JobID:   "JOB-GO-PDF",
				Format:  "pdf",
				FileURL: pdfURL,
				Copies:  1,
			},
			{
				JobID:   "JOB-GO-IMAGE",
				Format:  "image",
				FileURL: imageURL,
				Copies:  1,
			},
		},
	})
}

func (server *remoteTaskServer) handleStatusReport(writer http.ResponseWriter, request *http.Request) {
	var report map[string]any
	if err := json.NewDecoder(request.Body).Decode(&report); err != nil {
		writeJSON(writer, http.StatusBadRequest, map[string]string{"error": "invalid_json"})
		return
	}

	log.Printf("PrintBridge status report: %#v", report)
	writer.WriteHeader(http.StatusNoContent)
}

func handleConnectionTest(writer http.ResponseWriter, request *http.Request) {
	switch request.Method {
	case http.MethodGet, http.MethodPost:
		writer.WriteHeader(http.StatusNoContent)
	default:
		writeJSON(writer, http.StatusMethodNotAllowed, map[string]string{"error": "method_not_allowed"})
	}
}

func writeJSON(writer http.ResponseWriter, statusCode int, body any) {
	writer.Header().Set("Content-Type", "application/json; charset=utf-8")
	writer.WriteHeader(statusCode)
	if err := json.NewEncoder(writer).Encode(body); err != nil {
		log.Printf("failed to write JSON response: %v", err)
	}
}
