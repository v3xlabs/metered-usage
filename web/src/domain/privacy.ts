import { createSignal } from "solid-js";

const PRIVACY_STORAGE_KEY = "metered-usage-privacy";
// A fixed width, so the mask does not reveal how long the hidden text is.
const MASK = "********";

const [isPrivate, setIsPrivate] = createSignal(localStorage.getItem(PRIVACY_STORAGE_KEY) === "on");

export { isPrivate };

export const togglePrivacy = (): void => {
  const isNowPrivate = !isPrivate();

  setIsPrivate(isNowPrivate);
  localStorage.setItem(PRIVACY_STORAGE_KEY, isNowPrivate ? "on" : "off");
};

export const redact = (text: string): string => (isPrivate() ? MASK : text);
