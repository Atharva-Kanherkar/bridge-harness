import { act } from "react";

/**
 * Drive the Select primitive from a jsdom test the way a pointer would. The
 * trigger is the element carrying the select's aria-label; values are the
 * SelectItem `value`s, which the primitive mirrors to `data-value`.
 */
export function selectValue(trigger: HTMLElement): string {
  const hidden = document.getElementById(`${trigger.id}-hidden-input`) as HTMLInputElement | null;
  if (!hidden) throw new Error(`no Select behind ${trigger.outerHTML.slice(0, 120)}`);
  return hidden.value;
}

const fire = (target: HTMLElement, type: string) =>
  target.dispatchEvent(new (type.startsWith("pointer") ? PointerEvent : MouseEvent)(type, { bubbles: true, pointerType: "mouse" } as PointerEventInit));

/** The open listbox this trigger controls; a closing popup from an earlier select may still be mounted. */
function listbox(trigger: HTMLElement): HTMLElement | null {
  if (trigger.getAttribute("aria-expanded") !== "true") return null;
  const id = trigger.getAttribute("aria-controls");
  const owned = id ? document.getElementById(id) : null;
  return owned ?? [...document.querySelectorAll<HTMLElement>('[role="listbox"]')].find(box => !box.closest("[data-ending-style],[data-closed]")) ?? null;
}

/** Opens the popup; retries because the primitive ignores a press made while it settles. */
export async function openSelect(trigger: HTMLElement): Promise<HTMLElement | null> {
  for (let tries = 0; tries < 20 && !listbox(trigger); tries += 1) {
    if (trigger.getAttribute("aria-expanded") !== "true") {
      await act(async () => {
        fire(trigger, "pointerdown");
        fire(trigger, "mousedown");
        trigger.click();
      });
    }
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 10)); });
  }
  return listbox(trigger);
}

export async function chooseOption(trigger: HTMLElement, value: string): Promise<void> {
  await openSelect(trigger);
  const selector = `[role="option"][data-value="${value.replace(/["\\]/g, "\\$&")}"]`;
  const option = listbox(trigger)?.querySelector<HTMLElement>(selector) ?? null;
  if (!option) throw new Error(`no option "${value}" in ${trigger.getAttribute("aria-label") ?? "select"}`);
  await act(async () => {
    for (const type of ["pointerdown", "mousedown", "pointerup", "mouseup"]) fire(option, type);
    option.click();
    await new Promise(resolve => setTimeout(resolve, 0));
  });
}
