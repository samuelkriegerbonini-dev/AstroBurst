import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { ExternalLink, KeyRound, Loader2, LockOpen } from "lucide-react";
import { currentMission, romanPanelState, type ArchiveState } from "./state/archiveReducer";
import type { ArchiveController } from "./state/archiveController";
import {
  ACTION_LABELS,
  CHECKING_TITLE,
  ROMAN_NEEDS_TITLE,
  TOKEN_FIELD_LABEL,
  TOKEN_PAGE_NOTE,
  TOKEN_STORAGE_NOTE,
  TOKEN_SUBMIT_LABEL,
  romanPanelView,
  type RomanAction,
} from "./logic/romanPanel";
import { TOKEN_EXPIRY_TEXT } from "./logic/errorCopy";

type RomanCtl = Pick<
  ArchiveController,
  "saveToken" | "checkToken" | "cancelTokenCheck" | "clearToken" | "openTokenPage" | "replaceToken" | "setRomanPanel" | "tokenFocusDone"
>;

interface RomanPanelProps {
  state: ArchiveState;
  ctl: RomanCtl;
}

const TITLE_ID = "mast-roman-title";
const OUTCOME_ID = "mast-token-outcome";
const HINT_ID = "mast-token-hint";
const PAGE_NOTE_ID = "mast-token-page-note";
const INPUT_ID = "mast-token-input";

const ACTION_TESTIDS: Record<RomanAction, string> = {
  "open-page": "mast-token-open-page",
  check: "mast-token-check",
  replace: "mast-token-replace",
  remove: "mast-token-remove",
  cancel: "mast-token-cancel",
  "try-again": "mast-token-try-again",
  manage: "mast-token-manage",
  collapse: "mast-token-collapse",
  "cancel-replace": "mast-token-keep",
};

export default function RomanPanel({ state, ctl }: RomanPanelProps) {
  const panelRef = useRef<HTMLElement>(null);
  const titleRef = useRef<HTMLHeadingElement>(null);
  const outcomeRef = useRef<HTMLParagraphElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const [show, setShow] = useState(false);
  const [hasText, setHasText] = useState(false);
  const panelState = romanPanelState(state);
  const token = state.token;
  const view = romanPanelView({
    state: panelState,
    present: !!token.status?.present,
    message: token.message,
    checkedAt: token.checkedAt,
    replacing: token.replacing,
    expanded: token.expanded,
    checkedOn: currentMission(state)?.checkedOn ?? "",
  });
  const focus = token.focus;
  const formShown = view.form;

  useEffect(() => {
    if (!focus) return;
    const panel = panelRef.current;
    const active = document.activeElement;
    const free = !active || active === document.body || (!!panel && panel.contains(active));
    if (focus.target === "panel" || free) {
      const wantsField = focus.target === "field" || (focus.target === "panel" && formShown);
      const target = wantsField ? inputRef.current : focus.target === "outcome" ? outcomeRef.current : null;
      (target ?? titleRef.current ?? panel)?.focus();
    }
    ctl.tokenFocusDone(focus.seq);
  }, [focus, formShown, ctl]);

  const submit = () => {
    const el = inputRef.current;
    if (!el || token.checking) return;
    const raw = el.value;
    void ctl.saveToken(raw).finally(() => {
      if (inputRef.current) inputRef.current.value = "";
      setHasText(false);
      setShow(false);
    });
  };

  const run = (action: RomanAction) => {
    switch (action) {
      case "open-page":
        void ctl.openTokenPage();
        return;
      case "check":
        void ctl.checkToken();
        return;
      case "replace":
        ctl.replaceToken(true);
        return;
      case "remove":
        void ctl.clearToken();
        return;
      case "cancel":
        ctl.cancelTokenCheck();
        return;
      case "try-again":
        inputRef.current?.focus();
        return;
      case "manage":
        ctl.setRomanPanel(true);
        return;
      case "collapse":
        ctl.setRomanPanel(false);
        return;
      case "cancel-replace":
        ctl.replaceToken(false);
        return;
    }
  };

  const actionButton = (action: RomanAction) => (
    <button
      key={action}
      type="button"
      data-testid={ACTION_TESTIDS[action]}
      className="ab-btn-secondary ab-mast-action"
      aria-describedby={action === "open-page" ? PAGE_NOTE_ID : undefined}
      onClick={() => run(action)}
    >
      {action === "open-page" ? <ExternalLink size={11} aria-hidden="true" /> : null}
      {ACTION_LABELS[action]}
      {action === "cancel" ? <kbd className="ab-cmdp-kbd">Esc</kbd> : null}
    </button>
  );

  if (view.layout === "compact") {
    return (
      <section ref={panelRef} data-testid="mast-roman-panel" data-state={panelState} data-layout="compact" aria-label={view.title} className="ab-mast-roman ab-mast-roman-compact">
        <LockOpen size={12} aria-hidden="true" className="ab-mast-roman-ok" />
        <p ref={outcomeRef} id={OUTCOME_ID} data-tone={view.tone ?? undefined} tabIndex={-1} className="ab-mast-roman-outcome">
          {view.message}
        </p>
        <div className="ab-mast-roman-actions">{view.actions.map(actionButton)}</div>
      </section>
    );
  }

  const invalid = panelState === "rejected";
  const describedBy = invalid ? `${HINT_ID} ${OUTCOME_ID}` : HINT_ID;
  const onInputKey = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key !== "Enter") return;
    e.preventDefault();
    submit();
  };

  return (
    <section ref={panelRef} data-testid="mast-roman-panel" data-state={panelState} data-layout="full" aria-labelledby={TITLE_ID} className="ab-mast-roman">
      <h2 ref={titleRef} id={TITLE_ID} tabIndex={-1} className="ab-mast-roman-title">
        <KeyRound size={13} aria-hidden="true" />
        {view.title}
      </h2>
      {view.intro ? <p className="ab-mast-roman-text">{view.intro}</p> : null}
      {view.needs ? (
        <div className="ab-mast-roman-needs">
          <p className="ab-mast-roman-text">{ROMAN_NEEDS_TITLE}</p>
          <ol className="ab-mast-roman-list">
            {view.needs.map((need) => (
              <li key={need}>{need}</li>
            ))}
          </ol>
        </div>
      ) : null}
      {view.tokenLine ? <p className="ab-mast-roman-token">{view.tokenLine}</p> : null}
      {view.checking ? (
        <div className="ab-mast-roman-checking" role="status">
          <p className="ab-mast-roman-checking-title">
            <Loader2 size={12} aria-hidden="true" className="ab-mast-spin" />
            {CHECKING_TITLE}
          </p>
          <ol className="ab-mast-roman-list">
            {view.checking.map((step) => (
              <li key={step}>{step}</li>
            ))}
          </ol>
        </div>
      ) : null}
      {view.message ? (
        <p ref={outcomeRef} id={OUTCOME_ID} data-testid="mast-token-outcome" data-tone={view.tone ?? undefined} tabIndex={-1} className="ab-mast-roman-outcome">
          {view.message}
        </p>
      ) : null}
      {view.actions.length ? (
        <div className="ab-mast-roman-actions">
          {view.actions.map((action) =>
            action === "open-page" ? (
              <span key={action} className="ab-mast-roman-page">
                {actionButton(action)}
                <span id={PAGE_NOTE_ID} className="ab-mast-roman-note">
                  {`(${TOKEN_PAGE_NOTE})`}
                </span>
              </span>
            ) : (
              actionButton(action)
            ),
          )}
        </div>
      ) : null}
      {formShown ? (
        <div className="ab-mast-roman-form" role="group" aria-labelledby="mast-token-label">
          <label id="mast-token-label" htmlFor={INPUT_ID} className="ab-mast-roman-label">
            {TOKEN_FIELD_LABEL}
          </label>
          <div className="ab-mast-roman-field">
            <input
              ref={inputRef}
              id={INPUT_ID}
              data-testid="mast-token-input"
              type={show ? "text" : "password"}
              className="ab-mast-text ab-mast-roman-input"
              autoComplete="off"
              autoCapitalize="off"
              autoCorrect="off"
              spellCheck={false}
              aria-invalid={invalid}
              aria-describedby={describedBy}
              onInput={(e) => setHasText(e.currentTarget.value.trim().length > 0)}
              onKeyDown={onInputKey}
            />
            <button type="button" className="ab-btn-secondary ab-mast-roman-show" aria-pressed={show} aria-controls={INPUT_ID} onClick={() => setShow((v) => !v)}>
              {show ? "Hide" : "Show"}
            </button>
          </div>
          <p id={HINT_ID} className="ab-mast-roman-note">
            {TOKEN_EXPIRY_TEXT}
          </p>
          <button type="button" data-testid="mast-token-submit" className="ab-btn-primary ab-mast-action ab-mast-roman-submit" disabled={!hasText} onClick={submit}>
            {TOKEN_SUBMIT_LABEL}
          </button>
          <p className="ab-mast-roman-note">{TOKEN_STORAGE_NOTE}</p>
        </div>
      ) : null}
    </section>
  );
}
