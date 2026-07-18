package main

import (
	"bufio"
	"encoding/json"
	"os"
	"testing"
)

func TestRustCommandFixturesRemainGoCompatible(t *testing.T) {
	file, err := os.Open("../protocol/fixtures/commands-v2.jsonl")
	if err != nil {
		t.Fatal(err)
	}
	defer file.Close()

	count := 0
	scanner := bufio.NewScanner(file)
	for scanner.Scan() {
		var command map[string]any
		if err := json.Unmarshal(scanner.Bytes(), &command); err != nil {
			t.Fatalf("fixture line %d: %v", count+1, err)
		}
		if kind, ok := command["type"].(string); !ok || kind == "" {
			t.Fatalf("fixture line %d has no command type", count+1)
		}
		count++
	}
	if err := scanner.Err(); err != nil {
		t.Fatal(err)
	}
	if count != 59 {
		t.Fatalf("got %d command fixtures, want 59", count)
	}
}
