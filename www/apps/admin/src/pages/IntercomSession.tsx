import { useEffect, useState } from 'react';
import { useParams, useNavigate } from 'react-router-dom';
import { ChannelSession } from '@mediaservo/talk-ui';
import { getFloorSnapshot } from '../api/client';

/**
 * Intercom session page (talk-app T11): level-2 hold-to-talk for one room.
 * holdSecs comes from the REST floor limits (dispatch scope); the member list
 * is fed by the session connection's own room_peers frames (T6).
 */
export default function IntercomSession() {
  const { roomId = '' } = useParams();
  const navigate = useNavigate();
  const [holdSecs, setHoldSecs] = useState<number | null>(null);

  useEffect(() => {
    let alive = true;
    getFloorSnapshot(roomId)
      .then((snap) => {
        if (!alive) return;
        const limits = (snap as unknown as { limits?: { max_hold_secs?: number } }).limits;
        if (limits?.max_hold_secs) setHoldSecs(limits.max_hold_secs);
      })
      .catch(() => {
        /* limits stay at the default 30 s */
      });
    return () => {
      alive = false;
    };
  }, [roomId]);

  return (
    <section>
      <ChannelSession
        roomId={roomId}
        holdSecs={holdSecs ?? 30}
        onBack={() => navigate('/intercom')}
      />
    </section>
  );
}
