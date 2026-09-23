// A `date` input holds a bare calendar day and a `datetime-local` input a wall-clock time
// with no zone; the API speaks RFC3339 instants. Billing periods anchor on the UTC day, so
// day inputs map to UTC midnight, while a wall-clock input is read in the browser's zone.

export const utcDayOf = (timestamp: string): string => new Date(timestamp).toISOString()
  .slice(0, 10);

export const utcMidnight = (day: string): string => `${day}T00:00:00Z`;

export const localInstant = (wallClock: string): string => new Date(wallClock).toISOString();
