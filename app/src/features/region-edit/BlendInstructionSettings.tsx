// #55: the blend instruction text in Settings (app-wide, per machine). Saved with Save; "Reset to
// default" stores null (the core's default text). Maps also append "Top-down battlemap view.".
import { useEffect, useState } from 'react';

import { api, ApiError, type BlendInstruction } from '../../api';

export function BlendInstructionSettings() {
  const [loaded, setLoaded] = useState<BlendInstruction | null>(null);
  const [text, setText] = useState('');
  const [result, setResult] = useState<{ ok: boolean; text: string } | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    api.region.getBlendInstruction().then(
      (b) => {
        setLoaded(b);
        setText(b.text);
      },
      (e) => setResult({ ok: false, text: e instanceof Error ? e.message : String(e) }),
    );
  }, []);

  const store = async (value: string | null) => {
    setBusy(true);
    setResult(null);
    try {
      const b = await api.region.setBlendInstruction(value);
      setLoaded(b);
      setText(b.text);
      setResult({ ok: true, text: value === null ? 'Reset to the default.' : 'Saved.' });
    } catch (e) {
      setResult({ ok: false, text: e instanceof ApiError ? e.message : String(e) });
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="rgn-settings">
      <p className="prv-meta">
        Blend instruction: added to the prompt of a region edit while “Blend with surroundings” is
        on. Maps also add “Top-down battlemap view.”
      </p>
      <textarea
        className="rgn-prompt"
        aria-label="Blend instruction"
        data-testid="rgn-settings-blend-text"
        rows={6}
        value={text}
        disabled={!loaded}
        onChange={(e) => setText(e.target.value)}
      />
      <div className="prv-actions">
        <button
          type="button"
          className="primary"
          data-testid="rgn-settings-blend-save"
          disabled={!loaded || busy || text === loaded.text}
          onClick={() => void store(text)}
        >
          Save
        </button>
        <button
          type="button"
          data-testid="rgn-settings-blend-reset"
          disabled={!loaded || busy || (loaded.is_default && text === loaded.default_text)}
          onClick={() => void store(null)}
        >
          Reset to default
        </button>
      </div>
      {result && (
        <p
          className={result.ok ? 'prv-meta' : 'prv-meta prv-error'}
          data-testid="rgn-settings-blend-result"
        >
          {result.text}
        </p>
      )}
    </div>
  );
}
