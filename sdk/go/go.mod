module github.com/baals/sdk

go 1.21

// The BaaLS Go SDK is a pure-Go client library that communicates with a
// baalsd daemon process via HTTP endpoints and CLI commands.  No CGo or
// native linking is required.
//
// Build with:
//   go build ./...
//
// Example usage:
//   package main
//   import "github.com/baals/sdk"
//   func main() {
//       b, err := sdk.New("./data")
//       if err != nil {
//           panic(err)
//       }
//       defer b.Close()
//       b.Start()
//       state := b.ChainStateJSON()
//       fmt.Println(state)
//   }
