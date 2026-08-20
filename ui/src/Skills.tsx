import { useEffect, useState } from "react";
import type { SkillInventory } from "./api";
import { api, message } from "./api";

interface Props {
  profile: string;
  onClose: () => void;
  onError: (message: string) => void;
}

/** Everything a profile can call on.
 *
 *  Skills reach a profile two ways and the two have different levers: your own
 *  live in the profile and can be switched off one at a time, while plugin
 *  skills come with their plugin and can only be turned off together — that is
 *  the granularity Claude's settings offer, so it is the one shown. */
export function Skills({ profile, onClose, onError }: Props) {
  const [inventory, setInventory] = useState<SkillInventory | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [needsApply, setNeedsApply] = useState(false);
  const [expanded, setExpanded] = useState<string | null>(null);
  /** Removal asks twice: the button becomes its own confirmation. */
  const [confirming, setConfirming] = useState<string | null>(null);

  const load = () =>
    api
      .skills(profile)
      .then(setInventory)
      .catch((e) => onError(message(e)));

  useEffect(() => {
    void load();
  }, [profile]);

  async function act(id: string, work: () => Promise<unknown>) {
    setBusy(id);
    try {
      await work();
      await load();
    } catch (e) {
      onError(message(e));
    } finally {
      setBusy(null);
    }
  }

  if (!inventory) return null;

  const fromPlugins = inventory.plugins.filter((p) => p.skills > 0);
  const pluginTotal = fromPlugins.reduce((sum, p) => sum + p.skills, 0);

  return (
    <div className="sheet">
      <div className="sheet-head">
        <button className="btn quiet small" onClick={onClose}>
          ‹ Back
        </button>
        <div className="grow" />
        {needsApply && <span className="state">apply to take effect</span>}
      </div>

      <div className="sheet-body">
        <div className="group">
          <h3>In this profile</h3>
          {inventory.shared && (
            <p className="hint">
              This profile shares its skills directory, so turning one off turns it off
              everywhere. Give it its own copy first if that is not what you want.
            </p>
          )}
          {inventory.own.length === 0 && <p className="hint">No skills of its own.</p>}
          {confirming && (
            <p className="hint">
              Removing moves the skill to <span className="mono">skills.removed/</span> in this
              profile. Nothing is deleted.
            </p>
          )}

          {inventory.own.map((skill) => (
            <div className="skill" key={skill.name}>
              <div className="grow">
                <div className={`skill-name ${skill.enabled ? "" : "off"}`}>{skill.name}</div>
                {skill.description && <div className="skill-desc">{skill.description}</div>}
              </div>
              <div className="skill-actions">
                <button
                  className="btn small"
                  disabled={busy === skill.name}
                  onClick={() =>
                    act(skill.name, () =>
                      api.setSkillEnabled(profile, skill.name, !skill.enabled),
                    )
                  }
                >
                  {skill.enabled ? "Turn off" : "Turn on"}
                </button>
                {confirming === skill.name ? (
                  <button
                    className="btn small danger"
                    disabled={busy === skill.name}
                    onClick={() => {
                      setConfirming(null);
                      void act(skill.name, () => api.removeSkill(profile, skill.name));
                    }}
                  >
                    Confirm
                  </button>
                ) : (
                  <button
                    className="btn quiet small"
                    onClick={() => setConfirming(skill.name)}
                  >
                    Remove…
                  </button>
                )}
              </div>
            </div>
          ))}
        </div>

        {fromPlugins.length > 0 && (
          <div className="group">
            <h3>From plugins · {pluginTotal} skills</h3>
            <p className="hint">
              A plugin's skills come and go together — that is the only switch Claude has for
              them.
            </p>
            {fromPlugins.map((plugin) => (
              <div className="skill" key={plugin.key}>
                <div className="grow">
                  <div className={`skill-name ${plugin.enabled ? "" : "off"}`}>
                    {plugin.plugin}
                  </div>
                  <button
                    className="skill-desc link"
                    onClick={() => setExpanded(expanded === plugin.key ? null : plugin.key)}
                  >
                    {plugin.skills} skill{plugin.skills === 1 ? "" : "s"}
                    {expanded === plugin.key ? " ▾" : " ▸"}
                  </button>
                  {expanded === plugin.key && (
                    <div className="skill-names">{plugin.names.join(", ")}</div>
                  )}
                </div>
                <button
                  className="btn small"
                  disabled={busy === plugin.key}
                  onClick={() =>
                    act(plugin.key, async () => {
                      const apply = await api.setPluginEnabled(
                        profile,
                        plugin.key,
                        !plugin.enabled,
                      );
                      if (apply) setNeedsApply(true);
                    })
                  }
                >
                  {plugin.enabled ? "Turn off" : "Turn on"}
                </button>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
