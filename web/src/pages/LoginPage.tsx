import { useNavigate, useSearchParams } from "@solidjs/router";
import { createSignal, Show } from "solid-js";

import { returnPath, signIn } from "../api/session";
import { redact } from "../domain/privacy";

export const LoginPage = () => {
  const [searchParameters] = useSearchParams<{ next: string; }>();
  const navigate = useNavigate();
  const [failure, setFailure] = createSignal<string | null>(null);
  const [isSubmitting, setIsSubmitting] = createSignal(false);

  const submit = async (event: SubmitEvent & { currentTarget: HTMLFormElement; }): Promise<void> => {
    event.preventDefault();

    const password = new FormData(event.currentTarget).get("password");

    if (typeof password !== "string") return;

    setIsSubmitting(true);

    const result = await signIn(password);

    setIsSubmitting(false);

    if (!result.ok) {
      setFailure(result.message);

      return;
    }

    navigate(returnPath(searchParameters.next), { replace: true });
  };

  return (
    <form class="mx-auto max-w-sm space-y-4 rounded-panel bg-surface p-6" onSubmit={event => void submit(event)}>
      <h1 class="text-lg font-semibold">Sign in</h1>
      <div class="space-y-1">
        <label for="login-password" class="block text-xs font-medium text-slate-600 dark:text-slate-400">Password</label>
        <input
          id="login-password"
          name="password"
          type="password"
          required
          autocomplete="current-password"
          class="w-full rounded-control bg-raised px-3 py-1.5 text-sm text-slate-900 dark:text-slate-100"
        />
      </div>
      <Show when={failure()}>
        {message => <p class="text-sm text-red-600 dark:text-red-400" role="alert">{redact(message())}</p>}
      </Show>
      <button
        type="submit"
        disabled={isSubmitting()}
        class="w-full rounded-control bg-slate-900 px-3 py-1.5 text-sm font-medium text-white hover:bg-slate-700 disabled:opacity-60 dark:bg-slate-100 dark:text-slate-900 dark:hover:bg-white"
      >
        {isSubmitting() ? "Signing in..." : "Sign in"}
      </button>
    </form>
  );
};
