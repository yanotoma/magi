// Conversation state, shared across the panel.
//
// The `.svelte.ts` extension is REQUIRED for runes in a shared module — runes in
// a plain `.ts` file are a compile error.

export type Turn = {
  role: "user" | "assistant";
  content: string;
};

export const conversation = $state({
  turns: [] as Turn[],
  /** Tokens for the reply still arriving. Separate from `turns` so a cancelled
   *  turn can be discarded without touching the finished history. */
  streaming: null as string | null,
  error: null as string | null,
  notice: null as string | null,
});

export const startTurn = (text: string) => {
  conversation.error = null;
  conversation.notice = null;
  conversation.turns.push({ role: "user", content: text });
  conversation.streaming = "";
};

export const appendToken = (token: string) => {
  if (conversation.streaming === null) return;
  conversation.streaming += token;
};

/** Moves the streamed reply into history. */
export const finishTurn = (notice: string | null) => {
  if (conversation.streaming) {
    conversation.turns.push({ role: "assistant", content: conversation.streaming });
  }
  conversation.streaming = null;
  conversation.notice = notice;
};

export const failTurn = (message: string) => {
  // A partial answer is kept: whatever arrived before the failure is usually
  // still worth reading, and discarding it hides how far the model got.
  if (conversation.streaming) {
    conversation.turns.push({ role: "assistant", content: conversation.streaming });
  }
  conversation.streaming = null;
  conversation.error = message;
};

export const reset = () => {
  conversation.turns = [];
  conversation.streaming = null;
  conversation.error = null;
  conversation.notice = null;
};

/** History for the next request, excluding the turn being composed. */
export const historyForRequest = () =>
  conversation.turns.map((t) => ({ role: t.role, content: t.content }));
