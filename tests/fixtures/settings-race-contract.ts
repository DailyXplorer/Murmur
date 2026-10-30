import { emit } from "@tauri-apps/api/event";
import { mockIPC } from "@tauri-apps/api/mocks";
import type {
  AppSettings,
  BindingResponse,
  ShortcutBinding,
} from "../../src/bindings";
import {
  createSettingsChangedListenerLifecycle,
  useSettingsStore,
} from "../../src/stores/settingsStore";

const binding = (id: string, keys: string): ShortcutBinding => ({
  id,
  name: id,
  description: id,
  default_binding: "ctrl+space",
  current_binding: keys,
});
const INITIAL: AppSettings = {
  bindings: {
    transcribe: binding("transcribe", "ctrl+space"),
    cancel: binding("cancel", "escape"),
  },
  theme: "system",
  history_limit: 50,
};
const deferred = <T>() => {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
};
const tick = () =>
  new Promise<void>((resolve) => window.setTimeout(resolve, 0));
const waitFor = async (ready: () => boolean) => {
  const deadline = Date.now() + 3000;
  while (!ready()) {
    if (Date.now() > deadline)
      throw new Error("Probe did not reach its checkpoint");
    await tick();
  }
};
const accepted = (id: string, keys: string): BindingResponse => ({
  success: true,
  binding: binding(id, keys),
  error: null,
});
const rejected: BindingResponse = {
  success: false,
  binding: null,
  error: "Expected native registration refusal",
};

type ProbeResult = Record<string, string | number | boolean | undefined>;

/** One probe per fresh page/module. The Node regression runner loads this
 * same fixture with the real Tauri mock, generated bindings and Zustand. */
const run = async (scenario: string): Promise<ProbeResult> => {
  let backend = structuredClone(INITIAL);
  let reads = 0;
  let commandCalls = 0;
  const commandNames: string[] = [];
  const writes: Array<ReturnType<typeof deferred<BindingResponse>>> = [];
  const read = deferred<AppSettings>();
  let deferRead = scenario === "read-before-binding";
  const initialize = [
    "initial-event",
    "retry-snapshot",
    "shared-initialization",
    "retry-failures",
  ].includes(scenario);

  useSettingsStore.setState({
    settings: structuredClone(INITIAL),
    defaultSettings: structuredClone(INITIAL),
    isLoading: initialize,
    isUpdating: {},
  });

  mockIPC(
    (command) => {
      if (command === "get_app_settings") {
        reads++;
        const snapshot = structuredClone(backend);
        if (deferRead) {
          deferRead = false;
          return read.promise;
        }
        if (
          (scenario === "initial-event" || scenario === "retry-snapshot") &&
          reads === 1
        ) {
          return new Promise<AppSettings>((resolve) => {
            window.setTimeout(() => {
              backend.theme = "dark";
              void emit("settings-changed", { setting: "theme" }).then(() =>
                resolve(snapshot),
              );
            }, 0);
          });
        }
        return snapshot;
      }
      if (command === "get_default_settings") return structuredClone(INITIAL);
      if (command === "check_custom_sounds")
        return { start: false, stop: false };
      if (command === "change_binding" || command === "reset_binding") {
        commandCalls++;
        commandNames.push(command);
        if (scenario === "binding-ipc-refusal" && commandCalls === 1) {
          return Promise.reject("Expected IPC refusal");
        }
        const response = deferred<BindingResponse>();
        writes.push(response);
        return response.promise;
      }
      throw new Error(`Unexpected command: ${command}`);
    },
    { shouldMockEvents: true },
  );

  if (initialize) {
    let listens = 0;
    const internals = window.__TAURI_INTERNALS__;
    if (!internals) throw new Error("Tauri mock was not installed");
    const invoke = internals.invoke;
    internals.invoke = async (command, args) => {
      if (command === "plugin:event|listen") {
        listens++;
        if (
          (scenario === "retry-snapshot" && listens === 1) ||
          (scenario === "retry-failures" && listens <= 3)
        ) {
          throw "Expected listener registration failure";
        }
      }
      return invoke(command, args);
    };
    const first = useSettingsStore.getState().initialize();
    const second = useSettingsStore.getState().initialize();
    const third = useSettingsStore.getState().initialize();
    await Promise.all([first, second, third]);
    if (scenario === "retry-snapshot") await waitFor(() => listens === 2);
    if (scenario === "retry-failures") await waitFor(() => listens === 3);
    await tick();
    await tick();
    const result = {
      theme: useSettingsStore.getState().settings?.theme,
      reads,
      listens,
    };
    return scenario === "shared-initialization"
      ? { ...result, shared: first === second && second === third }
      : result;
  }

  if (scenario === "disposed-events") {
    let events = 0;
    let unlistens = 0;
    let handler:
      ((event: { payload: { setting?: string } }) => void) | undefined;
    const registration = deferred<() => void>();
    const lifecycle = createSettingsChangedListenerLifecycle({
      listen: (next) => {
        handler = next;
        return registration.promise;
      },
      onSettingsChanged: () => {
        events++;
      },
    });
    const pending = lifecycle.initialize();
    await tick();
    const cleanup = lifecycle.dispose();
    handler?.({ payload: { setting: "theme" } });
    registration.resolve(() => {
      unlistens++;
    });
    await Promise.all([pending, cleanup]);
    return { events, unlistens };
  }

  const setBackend = (id: string, keys: string) => {
    backend.bindings = { ...backend.bindings, [id]: binding(id, keys) };
  };
  const finish = (index: number, id: string, keys: string) => {
    setBackend(id, keys);
    writes[index].resolve(accepted(id, keys));
  };
  const current = () =>
    useSettingsStore.getState().settings?.bindings?.transcribe?.current_binding;

  if (scenario === "binding-before-refresh") {
    const write = useSettingsStore
      .getState()
      .updateBinding("transcribe", "ctrl+shift+x");
    await waitFor(() => commandCalls === 1);
    const refresh = useSettingsStore.getState().refreshSettings();
    await tick();
    const readsBeforeRelease = reads;
    finish(0, "transcribe", "Ctrl+Shift+X");
    await Promise.all([write, refresh]);
    return { keys: current(), readsBeforeRelease, reads };
  }

  if (scenario === "read-before-binding") {
    const refresh = useSettingsStore.getState().refreshSettings();
    await waitFor(() => reads === 1);
    const write = useSettingsStore
      .getState()
      .updateBinding("transcribe", "ctrl+shift+x");
    await waitFor(() => commandCalls === 1);
    finish(0, "transcribe", "Ctrl+Shift+X");
    await write;
    read.resolve(structuredClone(INITIAL));
    await refresh;
    return { keys: current(), reads };
  }

  if (scenario === "same-binding-failure") {
    const first = useSettingsStore
      .getState()
      .updateBinding("transcribe", "ctrl+shift+x");
    await waitFor(() => commandCalls === 1);
    const second = useSettingsStore
      .getState()
      .updateBinding("transcribe", "ctrl+shift+y")
      .then(
        () => true,
        () => false,
      );
    await tick();
    const callsBeforeRelease = commandCalls;
    useSettingsStore.setState((state) => ({
      settings: { ...state.settings, history_limit: 200 },
    }));
    finish(0, "transcribe", "Ctrl+Shift+X");
    await first;
    await waitFor(() => commandCalls === 2);
    const updatingBeforeRefusal = useSettingsStore
      .getState()
      .isUpdatingKey("binding_transcribe");
    writes[1].resolve(rejected);
    const secondAccepted = await second;
    const rolledBackKeys = current();
    // A refused command must not leave a rejected queue tail behind.
    const third = useSettingsStore
      .getState()
      .updateBinding("transcribe", "ctrl+shift+z");
    await waitFor(() => commandCalls === 3);
    finish(2, "transcribe", "Ctrl+Shift+Z");
    await third;
    return {
      callsBeforeRelease,
      updatingBeforeRefusal,
      secondAccepted,
      rolledBackKeys,
      keys: current(),
      historyLimit: useSettingsStore.getState().settings?.history_limit,
      updating: useSettingsStore.getState().isUpdatingKey("binding_transcribe"),
    };
  }

  if (scenario === "same-binding-first-refusal") {
    const first = useSettingsStore
      .getState()
      .updateBinding("transcribe", "ctrl+shift+x")
      .then(
        () => true,
        () => false,
      );
    await waitFor(() => commandCalls === 1);
    const second = useSettingsStore
      .getState()
      .updateBinding("transcribe", "ctrl+shift+y");
    await tick();
    const callsBeforeRelease = commandCalls;
    writes[0].resolve(rejected);
    const firstAccepted = await first;
    await waitFor(() => commandCalls === 2);
    const pendingKeys = current();
    const updatingBeforeAcceptance = useSettingsStore
      .getState()
      .isUpdatingKey("binding_transcribe");
    finish(1, "transcribe", "Ctrl+Shift+Y");
    await second;
    return {
      callsBeforeRelease,
      firstAccepted,
      pendingKeys,
      updatingBeforeAcceptance,
      keys: current(),
    };
  }

  if (scenario === "binding-ipc-refusal") {
    const firstAccepted = await useSettingsStore
      .getState()
      .updateBinding("transcribe", "ctrl+shift+x")
      .then(
        () => true,
        () => false,
      );
    const rolledBackKeys = current();
    const write = useSettingsStore
      .getState()
      .updateBinding("transcribe", "ctrl+shift+y");
    await waitFor(() => commandCalls === 2);
    finish(0, "transcribe", "Ctrl+Shift+Y");
    await write;
    return {
      firstAccepted,
      rolledBackKeys,
      keys: current(),
      updating: useSettingsStore.getState().isUpdatingKey("binding_transcribe"),
    };
  }

  if (scenario === "reset-after-update") {
    const write = useSettingsStore
      .getState()
      .updateBinding("transcribe", "ctrl+shift+x");
    await waitFor(() => commandCalls === 1);
    const reset = useSettingsStore.getState().resetBinding("transcribe");
    await tick();
    const callsBeforeRelease = commandCalls;
    finish(0, "transcribe", "Ctrl+Shift+X");
    await write;
    await waitFor(() => commandCalls === 2);
    const refresh = useSettingsStore.getState().refreshSettings();
    await tick();
    const readsBeforeRelease = reads;
    finish(1, "transcribe", "Ctrl+Space");
    await Promise.all([reset, refresh]);
    return {
      callsBeforeRelease,
      readsBeforeRelease,
      commands: commandNames.join(","),
      keys: current(),
    };
  }

  if (scenario === "independent-rollback") {
    const first = useSettingsStore
      .getState()
      .updateBinding("transcribe", "ctrl+shift+x")
      .then(
        () => true,
        () => false,
      );
    await waitFor(() => commandCalls === 1);
    const other = useSettingsStore.getState().updateBinding("cancel", "f8");
    await tick();
    const callsBeforeRelease = commandCalls;
    writes[0].resolve(rejected);
    const firstAccepted = await first;
    await waitFor(() => commandCalls === 2);
    finish(1, "cancel", "F8");
    await other;
    return {
      callsBeforeRelease,
      firstAccepted,
      keys: current(),
      otherKeys:
        useSettingsStore.getState().settings?.bindings?.cancel?.current_binding,
    };
  }

  if (scenario === "reset-refusal") {
    setBackend("transcribe", "ctrl+shift+r");
    useSettingsStore.setState({ settings: structuredClone(backend) });
    // The refresh below rereads the backend, so record what the rollback
    // itself published when the write settled.
    let rolledBackKeys: string | undefined;
    const unsubscribe = useSettingsStore.subscribe((state, previous) => {
      if (
        previous.isUpdating.binding_transcribe &&
        !state.isUpdating.binding_transcribe
      ) {
        rolledBackKeys = current();
      }
    });
    const reset = useSettingsStore.getState().resetBinding("transcribe");
    const pendingKeys = current();
    await waitFor(() => commandCalls === 1);
    const refresh = useSettingsStore.getState().refreshSettings();
    await tick();
    const readsBeforeRelease = reads;
    writes[0].resolve(rejected);
    await Promise.all([reset, refresh]);
    unsubscribe();
    return {
      pendingKeys,
      rolledBackKeys,
      keys: current(),
      readsBeforeRelease,
      updating: useSettingsStore.getState().isUpdatingKey("binding_transcribe"),
    };
  }
  throw new Error(`Unknown scenario: ${scenario}`);
};

declare global {
  interface Window {
    settingsRaceContract: { run: typeof run };
  }
}
window.settingsRaceContract = { run };
