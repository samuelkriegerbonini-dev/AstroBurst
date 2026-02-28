import { Component } from "react";
import { AlertTriangle, RefreshCw } from "lucide-react";

export default class ErrorBoundary extends Component {
  constructor(props) {
    super(props);
    this.state = { hasError: false, error: null };
  }

  static getDerivedStateFromError(error) {
    return { hasError: true, error };
  }

  componentDidCatch(error, errorInfo) {
    console.error("AstroKit Error:", error, errorInfo);
  }

  render() {
    if (this.state.hasError) {
      return (
        <div className="flex flex-col items-center justify-center h-screen bg-zinc-950 text-zinc-50 gap-6">
          <div className="flex items-center gap-3 text-red-500">
            <AlertTriangle size={48} />
          </div>
          <h1 className="text-2xl font-semibold">Something went wrong</h1>
          <p className="text-zinc-400 text-center max-w-md">
            AstroKit encountered an unexpected error. This might be a temporary
            issue. Try restarting the application.
          </p>
          <code className="text-xs text-zinc-500 font-mono bg-zinc-900 px-4 py-2 rounded-lg max-w-lg overflow-hidden text-ellipsis">
            {this.state.error?.message || "Unknown error"}
          </code>
          <button
            onClick={() => {
              this.setState({ hasError: false, error: null });
              window.location.reload();
            }}
            className="flex items-center gap-2 bg-blue-500 hover:bg-blue-600 text-white rounded-lg px-6 py-3 font-medium transition-colors"
          >
            <RefreshCw size={16} />
            Restart App
          </button>
        </div>
      );
    }

    return this.props.children;
  }
}
