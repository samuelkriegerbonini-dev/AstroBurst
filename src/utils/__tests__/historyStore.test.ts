import { beforeEach, describe, expect, it, vi } from "vitest";

const { announceMock } = vi.hoisted(() => ({ announceMock: vi.fn() }));

vi.mock("../../components/dock/useDockDrag", () => ({ announce: announceMock }));

import { EMPTY_HISTORY_SNAPSHOT, NO_IMAGE_MESSAGE, historyStore } from "../historyStore";

beforeEach(() => {
  historyStore.unbind();
  historyStore.set(EMPTY_HISTORY_SNAPSHOT);
  announceMock.mockReset();
});

describe("historyStore", () => {
  it("historyStore_actions_are_noops_until_bound", () => {
    expect(historyStore.isBound()).toBe(false);
    historyStore.undo();
    historyStore.redo();
    expect(announceMock).toHaveBeenCalledTimes(2);
    expect(announceMock).toHaveBeenCalledWith(NO_IMAGE_MESSAGE);

    const undo = vi.fn();
    const redo = vi.fn();
    historyStore.bind({ undo, redo });
    expect(historyStore.isBound()).toBe(true);
    historyStore.undo();
    historyStore.redo();
    historyStore.redo();
    expect(undo).toHaveBeenCalledTimes(1);
    expect(redo).toHaveBeenCalledTimes(2);
    expect(announceMock).toHaveBeenCalledTimes(2);

    historyStore.unbind();
    historyStore.undo();
    expect(undo).toHaveBeenCalledTimes(1);
    expect(announceMock).toHaveBeenCalledTimes(3);
  });

  it("a stale unbind leaves the newer binding in place", () => {
    const first = { undo: vi.fn(), redo: vi.fn() };
    const second = { undo: vi.fn(), redo: vi.fn() };
    const release = historyStore.bind(first);
    historyStore.bind(second);
    release();
    historyStore.undo();
    expect(second.undo).toHaveBeenCalledTimes(1);
    expect(first.undo).not.toHaveBeenCalled();
  });

  it("swallows a rejected action so a key press never throws", async () => {
    historyStore.bind({ undo: () => Promise.reject(new Error("boom")), redo: vi.fn() });
    expect(() => historyStore.undo()).not.toThrow();
    await Promise.resolve();
  });

  it("publishes snapshots to subscribers", () => {
    const listener = vi.fn();
    const stop = historyStore.subscribe(listener);
    const next = { ...EMPTY_HISTORY_SNAPSHOT, fileKey: "f|C:/a.fits" };
    historyStore.set(next);
    expect(historyStore.get()).toBe(next);
    historyStore.set(next);
    expect(listener).toHaveBeenCalledTimes(1);
    stop();
    historyStore.set(EMPTY_HISTORY_SNAPSHOT);
    expect(listener).toHaveBeenCalledTimes(1);
  });
});
