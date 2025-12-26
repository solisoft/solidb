package main

import (
	"fmt"
	"time"

	"github.com/solisoft/solidb-go-client/solidb"
)

func main() {
	client := solidb.NewClient("127.0.0.1", 9999)
	err := client.Connect()
	if err != nil {
		panic(err)
	}
	defer client.Close()

	client.Auth("_system", "admin", "admin")

	db := "bench_db"
	col := "go_bench"

	// Ignore errors if already exists
	client.CreateDatabase(db)
	client.CreateCollection(db, col, nil)

	iterations := 1000

	startTime := time.Now()
	for i := 0; i < iterations; i++ {
		data := map[string]interface{}{
			"id":   i,
			"data": "benchmark data content",
		}
		_, err = client.Insert(db, col, data, nil)
		if err != nil {
			fmt.Printf("Error: %v\n", err)
		}
	}
	duration := time.Since(startTime)

	opsPerSec := float64(iterations) / duration.Seconds()

	fmt.Printf("GO_BENCH_RESULT:%.2f\n", opsPerSec)
}
