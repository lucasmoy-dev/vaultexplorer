// hcshare: serves one folder over a zrok public share.
//
// zrok's own CLI would do this, but it carries a controller, a Postgres client
// and two web consoles, and comes to 96 MB — more than the whole rest of the
// Android app. This uses zrok's Go SDK directly and serves the folder with
// net/http, which is the entire job.
//
// Two commands, both meant to be driven by the app, never by a person:
//
//	hcshare enable <token>            joins the account, once per device
//	hcshare share <dir> <user:pass>   prints the URL, then serves until killed
package main

import (
	"encoding/json"
	"fmt"
	"net"
	"net/http"
	"os"
	"path/filepath"

	"github.com/openziti/zrok/v2/environment"
	"github.com/openziti/zrok/v2/sdk/golang/sdk"
)

func main() {
	if len(os.Args) < 2 {
		fail("usage: hcshare enable <token> | hcshare share <dir> [user:pass]")
	}
	switch os.Args[1] {
	case "enable":
		if len(os.Args) < 3 {
			fail("usage: hcshare enable <token>")
		}
		enable(os.Args[2])
	case "share":
		if len(os.Args) < 3 {
			fail("usage: hcshare share <dir> [user:pass]")
		}
		auth := ""
		if len(os.Args) > 3 {
			auth = os.Args[3]
		}
		share(os.Args[2], auth)
	case "status":
		status()
	default:
		fail("unknown command " + os.Args[1])
	}
}

// Joins this device to the account the token belongs to. Stored under
// ZROK_HOME, which the app points at its own directory so nothing lands in the
// user's real home.
func enable(token string) {
	root, err := environment.LoadRoot()
	if err != nil {
		fail("no se pudo abrir el entorno de zrok: " + err.Error())
	}
	if root.IsEnabled() {
		emit(map[string]any{"enabled": true, "already": true})
		return
	}
	host, _ := os.Hostname()
	if _, err := sdk.EnableEnvironment(root, &sdk.EnableRequest{
		Description: "HomeCloud on " + host,
	}); err != nil {
		fail(err.Error())
	}
	emit(map[string]any{"enabled": true})
}

// Whether this device has already joined an account.
func status() {
	root, err := environment.LoadRoot()
	if err != nil {
		emit(map[string]any{"enabled": false})
		return
	}
	emit(map[string]any{"enabled": root.IsEnabled()})
}

// Creates the public share, prints its URL as one line of JSON, and then serves
// the folder until the process is killed. Read-only: no upload, no delete.
func share(dir, auth string) {
	absolute, err := filepath.Abs(dir)
	if err != nil {
		fail(err.Error())
	}
	if info, err := os.Stat(absolute); err != nil || !info.IsDir() {
		fail("no es una carpeta: " + absolute)
	}

	root, err := environment.LoadRoot()
	if err != nil {
		fail("no se pudo abrir el entorno de zrok: " + err.Error())
	}
	if !root.IsEnabled() {
		fail("este dispositivo todavía no está conectado a una cuenta de zrok")
	}

	request := &sdk.ShareRequest{
		ShareMode:   sdk.PublicShareMode,
		BackendMode: sdk.ProxyBackendMode,
		Target:      "hcshare",
		NameSelections: []sdk.NameSelection{
			{NamespaceToken: "public"},
		},
	}
	if auth != "" {
		request.BasicAuth = []string{auth}
	}

	created, err := sdk.CreateShare(root, request)
	if err != nil {
		fail(err.Error())
	}
	// The app reads this line to know the URL. Everything else this process
	// writes is diagnostics.
	emit(map[string]any{
		"url":   firstEndpoint(created.FrontendEndpoints),
		"token": created.Token,
	})

	listener, err := sdk.NewListener(created.Token, root)
	if err != nil {
		fail(err.Error())
	}
	defer func() {
		_ = listener.Close()
		_ = sdk.DeleteShare(root, created)
	}()

	// http.FileServer already does directory listings and range requests, which
	// is what "browse the folder and download what you want" needs.
	server := &http.Server{Handler: readOnly(http.FileServer(http.Dir(absolute)))}
	if err := server.Serve(listener.(net.Listener)); err != nil {
		fail(err.Error())
	}
}

// Nothing but reads reaches the folder. The share is meant for handing photos
// to relatives, and an upload path nobody asked for is an upload path nobody
// is watching.
func readOnly(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodGet && r.Method != http.MethodHead {
			http.Error(w, "solo lectura", http.StatusMethodNotAllowed)
			return
		}
		next.ServeHTTP(w, r)
	})
}

func firstEndpoint(endpoints []string) string {
	if len(endpoints) == 0 {
		return ""
	}
	return endpoints[0]
}

func emit(value map[string]any) {
	line, _ := json.Marshal(value)
	fmt.Println(string(line))
}

func fail(message string) {
	line, _ := json.Marshal(map[string]any{"error": message})
	fmt.Fprintln(os.Stderr, string(line))
	os.Exit(1)
}
