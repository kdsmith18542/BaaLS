module github.com/baals/sdk

go 1.21

// The BaaLS Go SDK wraps the Rust FFI layer via CGo.
// Build with: CGO_ENABLED=1 go build -tags baals
//
// Example usage:
//   package main
//   import "github.com/baals/sdk"
//   func main() {
//       b, _ := sdk.New("./data")
//       b.Start()
//       state := b.ChainStateJSON()
//       println(state)
//   }
