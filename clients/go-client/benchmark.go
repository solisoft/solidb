package main

import (
	"fmt"
	"time"

	"github.com/solisoft/solidb-go-client/solidb"
)

func main() {
	client := solidb.NewClient("127.0.0.1", 9998)
	err := client.Connect()
	if err != nil {
		panic(err)
	}
	defer client.Close()

	client.Auth("_system", "admin", "password")

	db := "bench_db"
	col := "go_bench"

	// Ignore errors if already exists
	client.CreateDatabase(db)
	client.CreateCollection(db, col, nil)

	iterations := 1000

	// INSERT BENCHMARK
	startTime := time.Now()
	for i := 0; i < iterations; i++ {
		data := map[string]interface{}{
			"id":   i,
			"data": "benchmark data content",
		}
		key := fmt.Sprintf("bench_%d", i)
		_, err = client.Insert(db, col, data, &key)
		if err != nil {
			fmt.Printf("Error: %v\n", err)
		}
	}
	insertDuration := time.Since(startTime)
	insertOpsPerSec := float64(iterations) / insertDuration.Seconds()
	fmt.Printf("GO_BENCH_RESULT:%.2f\n", insertOpsPerSec)

	// READ BENCHMARK
	startTime = time.Now()
	for i := 0; i < iterations; i++ {
		key := fmt.Sprintf("bench_%d", i)
		_, err = client.Get(db, col, key)
		if err != nil {
			fmt.Printf("Read Error: %v\n", err)
		}
	}
	readDuration := time.Since(startTime)
	readOpsPerSec := float64(iterations) / readDuration.Seconds()
	fmt.Printf("GO_READ_BENCH_RESULT:%.2f\n", readOpsPerSec)
}
