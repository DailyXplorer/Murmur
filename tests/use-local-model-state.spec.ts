import { expect, test } from "@playwright/test";
import type { LocalModelStatus } from "../src/bindings";
import { loadComponent } from "./fixtures/load-component";

const preparing: LocalModelStatus = {
  state: { state: "preparing" },
  size_bytes: 1,
  kept_bytes: 0,
};
const ready: LocalModelStatus = { ...preparing, state: { state: "ready" } };

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

// Drain the asynchronous subscription and snapshot continuations deterministically.
const settle = async () => {
  for (let turn = 0; turn < 6; turn++) await Promise.resolve();
};

function mountHook() {
  let effect!: () => () => void;
  let listener!: (event: { payload: LocalModelStatus }) => void;
  let observed: LocalModelStatus | null = null;
  let backend = preparing;
  let readCount = 0;
  let stopCount = 0;
  let active = false;
  const writes: LocalModelStatus[] = [];
  const registration = deferred<() => void>();
  const snapshot = deferred<{
    status: "ok";
    data: LocalModelStatus;
  }>();
  let captured: LocalModelStatus;
  const { useLocalModel } = loadComponent<{
    useLocalModel: () => LocalModelStatus | null;
  }>(new URL("../src/hooks/useLocalModel.ts", import.meta.url), {
    react: {
      useState: () => [
        observed,
        (value: LocalModelStatus) => {
          observed = value;
          writes.push(value);
        },
      ],
      useEffect: (callback: () => () => void) => (effect = callback),
    },
    "@/bindings": {
      commands: {
        getLocalModelStatus: () => {
          readCount++;
          captured = backend;
          return snapshot.promise;
        },
      },
      events: {
        localModelStatus: {
          listen: (callback: typeof listener) => {
            listener = callback;
            return registration.promise;
          },
        },
      },
    },
  });
  expect(useLocalModel()).toBeNull();
  const unmount = effect();
  return {
    status: () => observed,
    readCount: () => readCount,
    stopCount: () => stopCount,
    writes,
    register: () => {
      active = true;
      registration.resolve(() => {
        active = false;
        stopCount++;
      });
    },
    failRegistration: () => registration.reject(new Error("listen failed")),
    transition: (status: LocalModelStatus) => {
      backend = status;
      if (active) listener({ payload: status });
    },
    queuedEvent: () => listener({ payload: ready }),
    resolveSnapshot: () => snapshot.resolve({ status: "ok", data: captured }),
    unmount,
  };
}

test("subscribes before reading and sees a transition during registration", async () => {
  const hook = mountHook();
  expect(hook.readCount()).toBe(0);
  hook.transition(ready); // No subscriber exists yet.
  hook.register();
  await settle();
  expect(hook.readCount()).toBe(1);
  hook.resolveSnapshot();
  await settle();
  expect(hook.status()).toEqual(ready);
  hook.unmount();
  await settle();
  expect(hook.stopCount()).toBe(1);
});

test("a late initial snapshot cannot overwrite a newer event", async () => {
  const hook = mountHook();
  hook.register();
  await settle(); // The initial snapshot captures Preparing.
  hook.transition(ready);
  hook.resolveSnapshot();
  await settle();
  expect(hook.status()).toEqual(ready);
  expect(hook.writes).toEqual([ready]);
  hook.unmount();
});

test("loads the initial snapshot and continues to receive events", async () => {
  const hook = mountHook();
  hook.register();
  await settle();
  hook.resolveSnapshot();
  await settle();
  expect(hook.status()).toEqual(preparing);
  hook.transition(ready);
  expect(hook.status()).toEqual(ready);
  hook.unmount();
  await settle();
  expect(hook.stopCount()).toBe(1);
});

test("a failed subscription still loads the snapshot", async () => {
  const hook = mountHook();
  hook.failRegistration();
  await settle();
  expect(hook.readCount()).toBe(1);
  hook.resolveSnapshot();
  await settle();
  expect(hook.status()).toEqual(preparing);
  hook.unmount();
});

test("unmount during registration releases the late listener without reading", async () => {
  const hook = mountHook();
  hook.unmount();
  hook.register();
  await settle();
  hook.queuedEvent();
  expect(hook.stopCount()).toBe(1);
  expect(hook.readCount()).toBe(0);
  expect(hook.writes).toEqual([]);
});

test("unmount during the snapshot ignores its result and queued events", async () => {
  const hook = mountHook();
  hook.register();
  await settle();
  hook.unmount();
  hook.queuedEvent();
  hook.resolveSnapshot();
  await settle();
  expect(hook.stopCount()).toBe(1);
  expect(hook.writes).toEqual([]);
});

test("Strict Mode remount does not let the disposed instance update state", async () => {
  const first = mountHook();
  first.unmount();
  const second = mountHook();
  second.register();
  await settle();
  first.register();
  await settle();
  first.queuedEvent();
  second.resolveSnapshot();
  await settle();
  expect(first.writes).toEqual([]);
  expect(first.stopCount()).toBe(1);
  expect(second.status()).toEqual(preparing);
  second.unmount();
  await settle();
  expect(second.stopCount()).toBe(1);
});
