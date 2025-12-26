package main

import (
	"fmt"
	"os"
	"strconv"
	"sync"
	"time"

	"github.com/solisoft/solidb-go-client/solidb"
)

func main() {
	portStr := os.Getenv("SOLIDB_PORT")
	if portStr == "" {
		portStr = "9998"
	}
	port, _ := strconv.Atoi(portStr)
	password := os.Getenv("SOLIDB_PASSWORD")
	if password == "" {
		password = "password"
	}

	numWorkers := 16
	totalInserts := 10000
	insertsPerWorker := totalInserts / numWorkers

	db := "bench_db"
	col := "go_parallel_bench"

	// Setup: create database and collection
	setupClient := solidb.NewClient("127.0.0.1", port)
	setupClient.Connect()
	setupClient.Auth("_system", "admin", password)
	setupClient.CreateDatabase(db)
	setupClient.CreateCollection(db, col, nil)
	setupClient.Close()

	var wg sync.WaitGroup
	startTime := time.Now()

	for w := 0; w < numWorkers; w++ {
		wg.Add(1)
		go func(workerID int) {
			defer wg.Done()

			client := solidb.NewClient("127.0.0.1", port)
			err := client.Connect()
			if err != nil {
				fmt.Printf("Worker %d: connection error: %v\n", workerID, err)
				return
			}
			defer client.Close()

			client.Auth("_system", "admin", password)

			for i := 0; i < insertsPerWorker; i++ {
				data := map[string]interface{}{
					"worker": workerID,
					"id":     i,
					"data":   "parallel benchmark data",
				}
				client.Insert(db, col, data, nil)
			}
		}(w)
	}

	wg.Wait()
	duration := time.Since(startTime)

	opsPerSec := float64(totalInserts) / duration.Seconds()
	fmt.Printf("GO_PARALLEL_BENCH_RESULT:%.2f\n", opsPerSec)
}
