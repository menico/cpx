import { useState } from "react";
import type { ProfileDetail as Detail, ResourceMode } from "./api";
import { api } from "./api";
import type { Ask } from "./Ask";

/** Eight identity colours, spaced far enough apart to tell apart at 3px wide. */
export const SWATCHES = [
  "#5c8dff",
  "#5dc794",
  "#d69552",
  "#c96ec9",
  "#5bbcd6",
  "#d4676b",
  "#9b8ce0",
  "#8a9099",
];

const MODES: { value: ResourceMode; label: string }[] = [
  { value: "link", label: "link" },
  { value: "copy", label: "copy" },
  { value: "own", label: "own" },
  { value: "merge", label: "merge" },
];

/** Plain-language versions of what each mode does, shown as a hint. */
const MODE_HELP: Record<ResourceMode, string> = {
  link: "Shared with ~/.claude — edit once, every profile sees it",
  copy: "Seeded once from ~/.claude, then this profile's own",
  own: "Private to this profile; ~/.claude is never read",
  merge: "Rebuilt from ~/.claude each time, with this profile's overrides on top",
};

interface Props {
  detail: Detail;
  onBack: () => void;
  onChanged: () => void;
  onError: (message: string) => void;
  onAsk: (ask: Ask) => void;
}

export function ProfileDetail({ detail, onBack, onChanged, onError, onAsk }: Props) {
  const { row } = detail;
  const [model, setModel] = useState(row.model ?? "");
  const [description, setDescription] = useState(row.description);

  async function guard(work: () => Promise<unknown>) {
    try {
      await work();
      onChanged();
    } catch (error) {
      onError(String(error));
    }
  }

  const setField = (field: string, value: string) =>
    guard(() => api.setField(row.name, field, value.trim() === "" ? null : value.trim()));

  return (
    <div className="sheet">
      <div className="sheet-head">
        <button className="btn quiet small" onClick={onBack}>
          ‹ Profiles
        </button>
        <div className="grow" />
        <button className="btn small" onClick={() => api.reveal(row.directory)}>
          Show in Finder
        </button>
      </div>

      <div className="sheet-body">
        <div className="group">
          {/* Not uppercased like the other headings: a profile name is a
              literal command, and `HD` is not a thing you can type. */}
          <h3 className="literal">{row.name}</h3>

          <div className="field">
            <label htmlFor="d-desc">Name</label>
            <input
              id="d-desc"
              type="text"
              value={description}
              placeholder="What this account is for"
              onChange={(e) => setDescription(e.target.value)}
              onBlur={() => setField("description", description)}
            />
          </div>

          <div className="field">
            <label>Colour</label>
            <div className="swatches">
              {SWATCHES.map((colour) => (
                <button
                  key={colour}
                  className="swatch"
                  style={{ background: colour }}
                  aria-pressed={row.color === colour}
                  aria-label={`Identity colour ${colour}`}
                  onClick={() => guard(() => api.setField(row.name, "color", colour))}
                />
              ))}
            </div>
          </div>

          <div className="field">
            <label htmlFor="d-model">Model</label>
            <input
              id="d-model"
              type="text"
              value={model}
              placeholder="Claude's default"
              onChange={(e) => setModel(e.target.value)}
              onBlur={() => setField("model", model)}
            />
          </div>
        </div>

        <div className="group">
          <h3>Account</h3>
          <div className="field">
            <label>Signed in as</label>
            <div className="value wrap">{row.account ?? "—"}</div>
          </div>
          <div className="field">
            <label>Token stored in</label>
            <div className="value">
              {row.credentialSource === "keychain"
                ? "macOS Keychain"
                : row.credentialSource === "file"
                  ? "credentials file"
                  : "not signed in"}
            </div>
          </div>
          <div className="field">
            <label />
            <div style={{ display: "flex", gap: 6 }}>
              <button
                className="btn small"
                disabled={!row.applied}
                onClick={() => guard(() => api.auth(row.name, row.signedIn ? "status" : "login"))}
              >
                {row.signedIn ? "Check account" : "Sign in…"}
              </button>
              {row.signedIn && (
                <button
                  className="btn small danger"
                  onClick={() => guard(() => api.auth(row.name, "logout"))}
                >
                  Sign out…
                </button>
              )}
            </div>
          </div>
          {!row.applied && (
            <div className="field">
              <label />
              <div className="value" style={{ fontFamily: "var(--sans)" }}>
                Apply your changes first — this profile has no directory yet.
              </div>
            </div>
          )}
        </div>

        <div className="group">
          <h3>What this profile gets</h3>
          {detail.resources.map((resource) => (
            <div className="resource" key={resource.resource} title={MODE_HELP[resource.mode]}>
              <span className="rname">{resource.resource}</span>
              {resource.hasPatch && <span className="patch">+patch</span>}
              <select
                value={resource.mode}
                aria-label={`How ${resource.resource} reaches this profile`}
                onChange={(e) =>
                  guard(() =>
                    api.setResource(row.name, resource.resource, e.target.value as ResourceMode),
                  )
                }
              >
                {MODES.filter((mode) => mode.value !== "merge" || resource.supportsMerge).map(
                  (mode) => (
                    <option key={mode.value} value={mode.value}>
                      {mode.label}
                    </option>
                  ),
                )}
              </select>
            </div>
          ))}
        </div>

        {detail.env.length > 0 && (
          <div className="group">
            <h3>Environment</h3>
            {detail.env.map(([key, value]) => (
              <div className="field" key={key}>
                <label>{key}</label>
                <div className="value wrap">{value}</div>
              </div>
            ))}
          </div>
        )}

        <div className="group">
          <h3>Details</h3>
          <div className="field">
            <label>Command</label>
            <div className="value">{row.command}</div>
          </div>
          <div className="field">
            <label>Directory</label>
            <div className="value wrap">{row.directory}</div>
          </div>
          <div className="field">
            <label>Keychain</label>
            <div className="value wrap">{detail.keychainService}</div>
          </div>
        </div>

        <div className="group">
          <button
            className="btn small"
            onClick={() =>
              onAsk({
                kind: "text",
                title: `Duplicate ${row.name}`,
                detail: "Copies its settings. The login is not copied — sign the new one in.",
                placeholder: `${row.name}-2`,
                submit: "Duplicate",
                onSubmit: (name) => guard(() => api.cloneProfile(row.name, name)),
              })
            }
          >
            Duplicate…
          </button>{" "}
          <button
            className="btn small danger"
            onClick={() =>
              onAsk({
                kind: "confirm",
                title: `Remove ${row.name}?`,
                detail: `Removes it from the config. ${row.directory} stays on disk, with its login intact.`,
                confirm: "Remove",
                danger: true,
                onConfirm: () =>
                  guard(async () => {
                    await api.removeProfile(row.name);
                    onBack();
                  }),
              })
            }
          >
            Remove…
          </button>
        </div>
      </div>
    </div>
  );
}
