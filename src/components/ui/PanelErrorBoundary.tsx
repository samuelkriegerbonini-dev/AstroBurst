import { Component, Fragment } from "react";
import type { ErrorInfo, ReactNode } from "react";
import { RefreshCw } from "lucide-react";
import { ERROR_ALERT_COMPACT_CLASS, SMALL_BUTTON_CLASS } from "./uiClasses";

export const PANEL_FAILED_PREFIX = "This panel failed: ";
export const RELOAD_PANEL_LABEL = "Reload panel";

interface PanelErrorBoundaryProps {
  name: string;
  children: ReactNode;
}

interface PanelErrorBoundaryState {
  error: Error | null;
  attempt: number;
}

export default class PanelErrorBoundary extends Component<PanelErrorBoundaryProps, PanelErrorBoundaryState> {
  constructor(props: PanelErrorBoundaryProps) {
    super(props);
    this.state = { error: null, attempt: 0 };
  }

  static getDerivedStateFromError(error: Error): Partial<PanelErrorBoundaryState> {
    return { error };
  }

  componentDidCatch(error: Error, errorInfo: ErrorInfo) {
    console.error(`[AstroBurst] ${this.props.name} panel failed:`, error, errorInfo);
  }

  reload = () => {
    this.setState({ error: null, attempt: this.state.attempt + 1 });
  };

  render() {
    const { error, attempt } = this.state;
    if (error) {
      return (
        <div className={`${ERROR_ALERT_COMPACT_CLASS} m-2 flex flex-col gap-2`} role="alert" data-panel-error={this.props.name}>
          <span>
            <strong>{this.props.name}</strong>
            {`. ${PANEL_FAILED_PREFIX}${error.message || "unknown error"}`}
          </span>
          <button type="button" className={`${SMALL_BUTTON_CLASS} self-start`} onClick={this.reload}>
            <RefreshCw size={10} />
            {RELOAD_PANEL_LABEL}
          </button>
        </div>
      );
    }
    return <Fragment key={attempt}>{this.props.children}</Fragment>;
  }
}
