## Required guarantees for quota refresh and reset

- No paid provider call runs without a human button press.
- No quota lookup or hard refresh runs without a human button press.
- Soft sync reads CLIProxy only.
- Cooldown resets name one account; nothing resets every account's cooldowns.
- Concurrent or recent requests for the same scope and action share one call.
- Action history holds no value derived from a token or password.
- Action history records the caller, outcome and quota values before and after each action.
- The change feed reports action updates.
- Live provider work survives caller disconnects and HTTP wait timeouts.
- An abandoned worker's action closes so the next request can run.
