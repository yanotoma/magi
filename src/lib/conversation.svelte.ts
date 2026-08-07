// Conversation state, shared across the panel.
//
// The `.svelte.ts` extension is REQUIRED for runes in a shared module — runes in
// a plain `.ts` file are a compile error.

export type Turn = {
  role: "user" | "assistant";
  content: string;
  /** The model's reasoning for this turn, when it emitted any. */
  thinking?: string;
};

export const conversation = $state({
  turns: [] as Turn[],
  /** The reply still arriving. Separate from `turns` so a cancelled turn can be
   *  discarded without touching finished history. `""` means the request is away
   *  but no token has landed yet, which is what the waiting indicator keys on. */
  streaming: null as string | null,
  /** Reasoning for the turn in flight. */
  thinking: "",
  error: null as string | null,
  notice: null as string | null,
});

export const startTurn = (text: string) => {
  conversation.error = null;
  conversation.notice = null;
  conversation.turns.push({ role: "user", content: text });
  conversation.streaming = "";
  conversation.thinking = "";
};

export const appendToken = (token: string) => {
  if (conversation.streaming === null) return;
  conversation.streaming += token;
};

export const appendThinking = (thought: string) => {
  if (conversation.streaming === null) return;
  conversation.thinking += thought;
};

/** Moves the streamed reply into history. */
export const finishTurn = (notice: string | null) => {
  commitStreamed();
  conversation.notice = notice;
};

export const failTurn = (message: string) => {
  // Whatever arrived before the failure is kept: it is usually still worth
  // reading, and discarding it hides how far the model got.
  commitStreamed();
  conversation.error = message;
};

const commitStreamed = () => {
  if (conversation.streaming) {
    conversation.turns.push({
      role: "assistant",
      content: conversation.streaming,
      thinking: conversation.thinking || undefined,
    });
  }
  conversation.streaming = null;
  conversation.thinking = "";
};

/** Resolves the local state after the user cancels.
 *
 *  Called by the panel rather than driven by an event, because there is no event
 *  to wait for: cancelling aborts the backend task, and an aborted task cannot
 *  emit a completion. The user initiated this, so the frontend already knows the
 *  turn is over — waiting for confirmation from a task that no longer exists is
 *  what left the panel stuck showing Stop with no way back. */
export const cancelStream = () => {
  commitStreamed();
  conversation.notice = null;
};

export const reset = () => {
  conversation.turns = [];
  conversation.streaming = null;
  conversation.thinking = "";
  conversation.error = null;
  conversation.notice = null;
};

/** History for the next request, excluding the turn being composed.
 *
 *  Reasoning is deliberately left out: it is context the model produced for
 *  itself, and replaying it as conversation would both cost tokens and invite the
 *  model to treat its own draft as established fact. */
export const historyForRequest = () =>
  conversation.turns.map((t) => ({ role: t.role, content: t.content }));
