import { useEffect, useRef, useState } from "react";

/** A question the interface needs answered.
 *
 *  `window.prompt` and `window.confirm` are unavailable in the app's webview —
 *  they return null without showing anything — so every question is asked in
 *  the interface itself. */
export type Ask =
  | {
      kind: "text";
      title: string;
      detail?: string;
      placeholder?: string;
      initial?: string;
      submit: string;
      onSubmit: (value: string) => void;
    }
  | {
      kind: "choose";
      title: string;
      detail?: string;
      options: { id: string; label: string; hint?: string; color?: string | null }[];
      onChoose: (id: string) => void;
    }
  | {
      kind: "confirm";
      title: string;
      detail?: string;
      confirm: string;
      danger?: boolean;
      onConfirm: () => void;
    };

export function AskSheet({ ask, onCancel }: { ask: Ask; onCancel: () => void }) {
  const [value, setValue] = useState(ask.kind === "text" ? (ask.initial ?? "") : "");
  const input = useRef<HTMLInputElement>(null);

  useEffect(() => {
    input.current?.focus();
    input.current?.select();
  }, [ask]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onCancel();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [onCancel]);

  const canSubmit = ask.kind !== "text" || value.trim() !== "";

  function submit() {
    if (ask.kind === "text" && canSubmit) ask.onSubmit(value.trim());
    if (ask.kind === "confirm") ask.onConfirm();
  }

  return (
    <div className="ask-backdrop" onClick={onCancel}>
      <div
        className="ask"
        role="dialog"
        aria-modal="true"
        aria-label={ask.title}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="ask-title">{ask.title}</div>
        {ask.detail && <div className="ask-detail">{ask.detail}</div>}

        {ask.kind === "text" && (
          <input
            ref={input}
            type="text"
            value={value}
            placeholder={ask.placeholder}
            onChange={(e) => setValue(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") submit();
            }}
          />
        )}

        {ask.kind === "choose" && (
          <div className="ask-options">
            {ask.options.map((option) => (
              <button
                className="ask-option"
                key={option.id}
                style={{ ["--bar" as string]: option.color ?? undefined }}
                onClick={() => ask.onChoose(option.id)}
              >
                <span className="bar" aria-hidden />
                <span className="grow">
                  <span className="name">{option.label}</span>
                  {option.hint && <span className="sub">{option.hint}</span>}
                </span>
              </button>
            ))}
          </div>
        )}

        <div className="ask-actions">
          <button className="btn small" onClick={onCancel}>
            Cancel
          </button>
          {ask.kind === "text" && (
            <button className="btn primary small" disabled={!canSubmit} onClick={submit}>
              {ask.submit}
            </button>
          )}
          {ask.kind === "confirm" && (
            <button
              className={`btn small ${ask.danger ? "danger" : "primary"}`}
              onClick={submit}
            >
              {ask.confirm}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
