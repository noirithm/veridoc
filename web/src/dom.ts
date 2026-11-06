export function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  text?: string,
  className?: string,
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  // textContent keeps file names and findings from being treated as HTML.
  if (text !== undefined) node.textContent = text;
  if (className) node.className = className;
  return node;
}

export function bulletList(items: string[]): HTMLUListElement {
  const list = el('ul');
  for (const item of items) {
    list.append(el('li', item));
  }
  return list;
}