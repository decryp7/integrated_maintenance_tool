import { dialogKeyAction, DialogKeyEvent, findDefaultAction } from './dialog-keys';

const ev = (key: string, over: Partial<DialogKeyEvent> = {}): DialogKeyEvent => ({ key, metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, target: { tagName: 'INPUT' }, ...over });

describe('dialogKeyAction', () => {
  it('closes on Escape and on Cmd/Ctrl+W', () => {
    expect(dialogKeyAction(ev('Escape'))).toBe('close');
    expect(dialogKeyAction(ev('w', { metaKey: true }))).toBe('close');
    expect(dialogKeyAction(ev('W', { ctrlKey: true }))).toBe('close');
    expect(dialogKeyAction(ev('w'))).toBeNull();
    expect(dialogKeyAction(ev('w', { metaKey: true, shiftKey: true }))).toBeNull();
    expect(dialogKeyAction(ev('Escape', { ctrlKey: true }))).toBeNull();
  });

  it('runs the default action on a plain Enter, except where Enter means something else', () => {
    expect(dialogKeyAction(ev('Enter'))).toBe('default');
    expect(dialogKeyAction(ev('Enter', { target: { tagName: 'DIV' } }))).toBe('default');
    for (const tagName of ['TEXTAREA', 'BUTTON', 'A', 'SELECT', 'SUMMARY']) expect(dialogKeyAction(ev('Enter', { target: { tagName } }))).toBeNull();
    expect(dialogKeyAction(ev('Enter', { target: { tagName: 'DIV', isContentEditable: true } }))).toBeNull();
    expect(dialogKeyAction(ev('Enter', { target: { tagName: 'LI', getAttribute: () => 'option' } }))).toBeNull();
    expect(dialogKeyAction(ev('Enter', { shiftKey: true }))).toBeNull();
    expect(dialogKeyAction(ev('Enter', { metaKey: true }))).toBeNull();
    expect(dialogKeyAction(ev('Enter', { target: null }))).toBe('default');
  });

  it('ignores composition and keys that were already handled', () => {
    expect(dialogKeyAction(ev('Enter', { isComposing: true }))).toBeNull();
    expect(dialogKeyAction(ev('Escape', { defaultPrevented: true }))).toBeNull();
    expect(dialogKeyAction(ev('a'))).toBeNull();
  });
});

describe('findDefaultAction', () => {
  const root = (html: string) => {
    const d = document.createElement('div');
    d.innerHTML = html;
    return d;
  };
  it('prefers [data-default], then the first enabled primary button', () => {
    expect(findDefaultAction(root('<button class="btn primary" id="p">x</button><button data-default id="d">y</button>'))?.id).toBe('d');
    expect(findDefaultAction(root('<button class="btn primary" disabled id="a">x</button><button class="btn primary" id="b">y</button>'))?.id).toBe('b');
    expect(findDefaultAction(root('<button class="btn">x</button>'))).toBeNull();
    expect(findDefaultAction(root('<button data-default disabled>x</button>'))).toBeNull();
  });
});
