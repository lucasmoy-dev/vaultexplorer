// hcshare: serves one folder as a temporary public link over a Cloudflare
// Quick Tunnel.
//
// No account anywhere. A quick tunnel needs none: this process dials out to
// Cloudflare and gets back a random https://something.trycloudflare.com
// address that forwards back down the same connection. Nobody connects
// inward, which is what makes it work on a phone with no address of its own.
//
// The password on the link — if there is one — is a plain HTTP Basic Auth
// this process checks itself. Nothing is handed to a third party to compare
// against what a visitor types.
//
// The link is deliberately temporary, not a bug worth working around: a quick
// tunnel gets a new address every time and Cloudflare promises no uptime, so
// treating it as permanent would be a lie the app tells on the user's behalf.
// --for bounds how long the folder stays reachable; the process exits on its
// own when that runs out, which is also what happens if it is killed early.
//
// One command, meant to be driven by the app, never typed by a person:
//
//	hcshare share <dir> <cloudflared-binary> [user:pass] [--for=6h]
package main

import (
	"bufio"
	"context"
	"encoding/json"
	"fmt"
	"net"
	"net/http"
	"os"
	"os/exec"
	"os/signal"
	"path/filepath"
	"regexp"
	"strings"
	"syscall"
	"time"
)

var tunnelURL = regexp.MustCompile(`https://[a-zA-Z0-9-]+\.trycloudflare\.com`)

// How long a link lasts when nothing else is said. Long enough to hand
// someone a link and have them get to it that evening, short enough that a
// forgotten link does not sit open for a week.
const defaultDuration = 6 * time.Hour

// How long cloudflared gets to announce a URL before this gives up on it.
const announceTimeout = 45 * time.Second

func main() {
	if len(os.Args) < 4 || os.Args[1] != "share" {
		fail("usage: hcshare share <dir> <cloudflared-binary> [user:pass] [--for=6h]")
	}
	dir := os.Args[2]
	cloudflaredBin := os.Args[3]

	auth := ""
	duration := defaultDuration
	for _, arg := range os.Args[4:] {
		if rest, ok := strings.CutPrefix(arg, "--for="); ok {
			if parsed, err := time.ParseDuration(rest); err == nil && parsed > 0 {
				duration = parsed
			}
			continue
		}
		auth = arg
	}

	share(dir, cloudflaredBin, auth, duration)
}

func share(dir, cloudflaredBin, auth string, duration time.Duration) {
	absolute, err := filepath.Abs(dir)
	if err != nil {
		fail(err.Error())
	}
	if info, err := os.Stat(absolute); err != nil || !info.IsDir() {
		fail("no es una carpeta: " + absolute)
	}

	// Port 0: the OS picks one that is actually free, so two shares on the
	// same device never fight over a fixed number.
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		fail("no se pudo abrir un puerto local: " + err.Error())
	}
	port := listener.Addr().(*net.TCPAddr).Port

	var handler http.Handler = readOnly(http.FileServer(http.Dir(absolute)))
	if auth != "" {
		user, pass, ok := strings.Cut(auth, ":")
		if !ok {
			fail("la contraseña debe ir como usuario:contraseña")
		}
		handler = requireBasicAuth(handler, user, pass)
	}
	server := &http.Server{Handler: handler}
	go func() { _ = server.Serve(listener) }()

	ctx, cancel := context.WithTimeout(context.Background(), duration)
	defer cancel()

	cmd := exec.CommandContext(
		ctx, cloudflaredBin,
		"tunnel", "--url", fmt.Sprintf("http://127.0.0.1:%d", port), "--no-autoupdate",
	)
	// cloudflared logs its own address to stderr, not stdout.
	stderr, err := cmd.StderrPipe()
	if err != nil {
		fail("no se pudo lanzar cloudflared: " + err.Error())
	}
	if err := cmd.Start(); err != nil {
		fail("no se pudo lanzar cloudflared: " + err.Error())
	}

	found := make(chan string, 1)
	go func() {
		scanner := bufio.NewScanner(stderr)
		for scanner.Scan() {
			if url := tunnelURL.FindString(scanner.Text()); url != "" {
				select {
				case found <- url:
				default:
				}
			}
		}
	}()

	var url string
	select {
	case url = <-found:
	case <-time.After(announceTimeout):
		fail("cloudflared no contestó a tiempo")
	case <-ctx.Done():
		fail("se acabó el tiempo antes de conseguir el enlace")
	}

	// The app reads this line to know the URL and when to stop trusting it.
	// Everything printed after this is diagnostics, not part of the contract.
	emit(map[string]any{
		"url":       url,
		"expiresAt": time.Now().Add(duration).Unix(),
	})

	// Runs until the duration is up, cloudflared dies on its own, or the
	// parent process asks nicely first.
	stop := make(chan os.Signal, 1)
	signal.Notify(stop, syscall.SIGTERM, syscall.SIGINT)
	select {
	case <-ctx.Done():
	case <-stop:
	}
	_ = server.Close()
}

// Nothing but reads reaches the folder. The link is meant for handing photos
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

// Checked here, by this process, against the password the app was told to
// use — never handed to cloudflared or to anything of Cloudflare's. A quick
// tunnel has no account behind it to check a password on their end even if
// we wanted to.
func requireBasicAuth(next http.Handler, wantUser, wantPass string) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		user, pass, ok := r.BasicAuth()
		if !ok || user != wantUser || pass != wantPass {
			w.Header().Set("WWW-Authenticate", `Basic realm="HomeCloud"`)
			http.Error(w, "hace falta la contraseña", http.StatusUnauthorized)
			return
		}
		next.ServeHTTP(w, r)
	})
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
