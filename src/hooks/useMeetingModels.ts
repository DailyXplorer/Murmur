import { useEffect, useState } from "react";
import { commands, events, type MeetingModelsStatus } from "@/bindings";

/** The speaker models' state, kept current while they download. `null`
 * until it is known. */
export const useMeetingModels = (): MeetingModelsStatus | null => {
  const [status, setStatus] = useState<MeetingModelsStatus | null>(null);

  useEffect(() => {
    let disposed = false;
    void commands.getMeetingModelsStatus().then((result) => {
      if (!disposed && result.status === "ok") setStatus(result.data);
    });
    const unlisten = events.meetingModelsStatus.listen((event) => {
      setStatus(event.payload);
    });
    return () => {
      disposed = true;
      void unlisten.then((stop) => stop());
    };
  }, []);

  return status;
};
