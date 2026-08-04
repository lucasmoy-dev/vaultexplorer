import { Component, ErrorInfo, ReactNode } from "react";

// A crash during render unmounts the whole tree, and this window is
// `transparent: true` with no decorations -- so the result isn't a browser
// error page, it's a blank pane with nothing to click and no way to tell what
// happened. This catches that and shows the error plus a way back, which also
// means a reproducible crash reports itself instead of being described as
// "the screen goes white".
export class ErrorBoundary extends Component<
  { children: ReactNode },
  { error: Error | null; stack: string }
> {
  state = { error: null as Error | null, stack: "" };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    // Kept for the details block below -- componentStack names the component
    // that actually threw, which the message alone usually doesn't.
    this.setState({ error, stack: info.componentStack ?? "" });
    console.error("VaultExplorer crashed while rendering:", error, info.componentStack);
  }

  render() {
    const { error, stack } = this.state;
    if (!error) return this.props.children;
    return (
      <div className="crash-pane">
        <h3>Something broke while drawing this view</h3>
        <p className="crash-msg">{error.message || String(error)}</p>
        <div className="sheet-actions">
          <button className="btn-plain" onClick={() => this.setState({ error: null, stack: "" })}>
            Try Again
          </button>
          <button className="btn-primary" onClick={() => window.location.reload()}>
            Reload
          </button>
        </div>
        <details className="crash-details">
          <summary>Details</summary>
          <pre>
            {error.stack || ""}
            {stack}
          </pre>
        </details>
      </div>
    );
  }
}
