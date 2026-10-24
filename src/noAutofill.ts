/**
 * O WebView2 oferece guardar e repetir o que se digita nos campos ("As informações foram salvas"). O Atlas tem o próprio
 * histórico e cofre de senhas, então desliga isso em todo campo que não tenha escolhido outro valor.
 */
const FIELDS = "input:not([autocomplete]), textarea:not([autocomplete])";

function off(root: ParentNode) {
  for (const el of root.querySelectorAll(FIELDS)) el.setAttribute("autocomplete", "off");
}

export function disableAutofill() {
  off(document);
  new MutationObserver((records) => {
    for (const r of records) {
      for (const n of r.addedNodes) {
        if (!(n instanceof Element)) continue;
        if (n.matches(FIELDS)) n.setAttribute("autocomplete", "off");
        off(n);
      }
    }
  }).observe(document.body, { childList: true, subtree: true });
}
