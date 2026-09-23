import { api } from "./client";
import type { components } from "./schema.gen";

export type DeadLetter = components["schemas"]["DeadLetter"];

export const listDeadLetters = async (sourceId: string): Promise<readonly DeadLetter[]> => {
  const response = await api("/dead-letters", "get", { query: { source_id: sourceId } });

  if (response.status === 200) return response.data.dead_letters;

  throw new Error(`Dead letter list failed with status ${response.status}.`);
};
