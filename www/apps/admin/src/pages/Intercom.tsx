import { Button, Guard, StatusGlyph } from '@mediaservo/react';
import { VERSION as TALK_UI_VERSION } from '@mediaservo/talk-ui';

/**
 * Intercom placeholder (talk-app T0).
 * Proves the package chain admin -> talk-ui -> react -> ui tokens.
 * The real floor board and channel session land with talk-app batches
 * T5 (board rows) and T11 (session view) — see docs/plans/talk-app.
 */
export default function Intercom() {
  return (
    <section>
      <h1>Intercom</h1>
      <p>
        Placeholder — the floor board and channel session arrive with talk-app
        batches T5/T11 (talk-ui v{TALK_UI_VERSION}). State legend preview:
      </p>
      <p>
        <StatusGlyph tone="ok" shape="dot" label="held" /> held{' '}
        <StatusGlyph tone="warn" shape="seg" label="queued" /> queued{' '}
        <StatusGlyph tone="off" shape="ring" label="idle" /> idle{' '}
        <StatusGlyph tone="unknown" shape="triangle" label="unknown — reconnecting" /> unknown
      </p>
      <p>
        <Guard reason="Board actions land with batch T5 (floor capability: dispatcher + admin)">
          <Button variant="primary">Grant (preview)</Button>
        </Guard>{' '}
        <Button variant="secondary" disabled disabledReason="Session view lands with batch T11">
          Join (preview)
        </Button>
      </p>
    </section>
  );
}
