// Which IPC commands can a piece of the UI reach? A static call-graph walk over the TypeScript program (type-checker
// resolved, so a call through an injected abstract backend, a SignalStore method or a DI token is followed to the code
// that runs). Used by `check-dialog-commands.mjs` to derive every dialog's command list and compare it, in both
// directions, with the list declared in the registry (ADR 0054). Over-approximates on purpose: a reference to a
// function counts as a call, a component counts with all its members, an injected class with its constructor.
import { readFileSync } from 'node:fs';
import { dirname, join, resolve, sep } from 'node:path';
import ts from 'typescript';

/** A command name as written at a call site: `invoke('x')`, `this.call('x')`, `this.invoke<T>('x')`. */
const COMMAND = /^[a-z][a-z0-9_]*$/;
const CALLEES = new Set(['invoke', 'call']);

/** A TypeScript program over `tsconfig` (the app's, or a small one in a test). */
export function createAnalysis(tsconfigPath) {
  const cfg = ts.getParsedCommandLineOfConfigFile(tsconfigPath, {}, { ...ts.sys, onUnRecoverableConfigFileDiagnostic: () => undefined });
  if (!cfg) throw new Error(`cannot read ${tsconfigPath}`);
  const program = ts.createProgram(cfg.fileNames, cfg.options);
  const checker = program.getTypeChecker();
  const own = (sf) => !sf.isDeclarationFile && !sf.fileName.includes(`${sep}node_modules${sep}`) && !sf.fileName.includes('/node_modules/');
  const direct = indexImplementers(program, checker, own);
  // Every class that extends or implements `c`, and their subclasses in turn (`ToolsBackend implements LifecycleBackend`,
  // `TauriToolsBackend extends ToolsBackend`: the last one has the code).
  const implementers = {
    get(c) {
      const out = [];
      const todo = [...(direct.get(c) ?? [])];
      while (todo.length) {
        const x = todo.pop();
        if (out.includes(x)) continue;
        out.push(x);
        todo.push(...(direct.get(x) ?? []));
      }
      return out;
    },
  };
  const { objectImpls, lazyLiterals } = indexObjectImplementations(program, checker, own);

  const stripAlias = (symbol) => (symbol && symbol.flags & ts.SymbolFlags.Alias ? checker.getAliasedSymbol(symbol) : symbol);
  const declsOf = (n) => {
    let s = stripAlias(checker.getSymbolAtLocation(n));
    if (!s?.declarations?.length && ts.isPropertyAccessExpression(n.parent) && n.parent.name === n) {
      // A member of a mapped / intersection type (a SignalStore's methods): the property of the receiver's type.
      const t = checker.getTypeAtLocation(n.parent.expression);
      s = t.getProperty?.(n.text) ?? s;
    }
    return (s?.declarations ?? []).filter((d) => own(d.getSourceFile()));
  };

  /**
   * The commands reachable from `entries` (nodes): a Map command -> the source location that calls it, and the
   * dynamic call sites it could not resolve. `known` is the set of command names (the Rust table): a call whose first
   * argument is one of them is a command call, whatever the wrapper (`call`, `invoke`, `run`) is named. `boundary`
   * is a set of declarations the walk does not enter (their commands are listed by hand and named in the registry).
   */
  function reach(entries, known, boundary = new Set()) {
    const seen = new Set();
    const queue = [];
    const found = new Map();
    const dynamic = [];
    const inBoundary = (n) => {
      for (let x = n; x; x = x.parent) if (boundary.has(x)) return true;
      return false;
    };
    let current = null;
    const foundIn = new Map();
    const via = new Map();
    const push = (n) => {
      if (n && !seen.has(n) && !inBoundary(n)) {
        seen.add(n);
        via.set(n, current);
        queue.push(n);
      }
    };
    const seeds = [];
    for (const e of entries) (e.decl ? seeds.push(e.decl) : push(e));

    const where = (n) => {
      const sf = n.getSourceFile();
      return `${sf.fileName}:${sf.getLineAndCharacterOfPosition(n.getStart()).line + 1}`;
    };

    // An initializer runs when the instance is made if it calls something; a plain alias (`summary = this.data.summary`) or
    // a function value does not run anything: it is followed when the property is used.
    const runsOnConstruction = (m) => {
      if (!m.initializer || m.modifiers?.some((x) => x.kind === ts.SyntaxKind.StaticKeyword)) return false;
      if (ts.isArrowFunction(m.initializer) || ts.isFunctionExpression(m.initializer)) return false;
      let calls = false;
      const look = (n) => {
        if (ts.isCallExpression(n) || ts.isNewExpression(n) || ts.isTaggedTemplateExpression(n)) calls = true;
        if (!calls && !ts.isArrowFunction(n) && !ts.isFunctionExpression(n)) ts.forEachChild(n, look);
      };
      look(m.initializer);
      return calls;
    };
    const constructed = new Set();
    const construct = (cls) => {
      if (constructed.has(cls)) return;
      constructed.add(cls);
      // Injecting a class runs its constructor and property initializers; so does its base class, and (abstract
      // class used as a token) every implementation.
      for (const c of [cls, ...(implementers.get(cls) ?? [])]) {
        constructed.add(c);
        for (const m of c.members) {
          if (ts.isConstructorDeclaration(m) || (ts.isPropertyDeclaration(m) && runsOnConstruction(m))) push(m);
        }
        for (const h of c.heritageClauses ?? []) {
          if (h.token !== ts.SyntaxKind.ExtendsKeyword) continue;
          for (const t of h.types) for (const d of declsOf(t.expression)) if (ts.isClassDeclaration(d)) construct(d);
        }
      }
    };

    const follow = (decl, injected) => {
      if (ts.isClassDeclaration(decl)) {
        if (injected) construct(decl);
      } else if (ts.isVariableDeclaration(decl)) {
        if (decl.initializer) push(decl.initializer);
      } else if (ts.isMethodDeclaration(decl) || ts.isMethodSignature(decl) || ts.isPropertySignature(decl) || ts.isPropertyDeclaration(decl) || ts.isGetAccessor(decl) || ts.isSetAccessor(decl)) {
        push(decl);
        // The method of an abstract class / interface: every implementation's member of that name.
        const cls = decl.parent;
        const name = decl.name?.getText?.();
        for (const impl of implementers.get(cls) ?? []) {
          for (const m of impl.members) if (m.name?.getText?.() === name) push(m);
        }
        // An object literal returned by a factory typed as this interface: its member of that name.
        for (const lit of objectImpls.get(cls) ?? []) {
          for (const m of lit.properties) {
            if (m.name?.getText?.() !== name) continue;
            if (ts.isPropertyAssignment(m)) push(m.initializer);
            else if (ts.isMethodDeclaration(m)) push(m);
            else if (ts.isShorthandPropertyAssignment(m)) follow(m, false);
          }
        }
      } else if (ts.isPropertyAssignment(decl)) {
        push(decl.initializer);
      } else if (ts.isShorthandPropertyAssignment(decl)) {
        const value = stripAlias(checker.getShorthandAssignmentValueSymbol(decl));
        for (const d of value?.declarations ?? []) if (own(d.getSourceFile()) && d !== decl) follow(d, false);
      } else if (ts.isFunctionDeclaration(decl) || ts.isFunctionExpression(decl) || ts.isArrowFunction(decl)) {
        push(decl);
      } else if (ts.isParameter(decl)) {
        // A constructor parameter property of an injected class: nothing to run.
      }
    };

    const command = (call) => {
      if (call.arguments.length === 0) return;
      const callee = call.expression;
      const name = ts.isIdentifier(callee) ? callee.text : ts.isPropertyAccessExpression(callee) ? callee.name.text : '';
      const arg = call.arguments[0];
      if (ts.isStringLiteralLike(arg)) {
        if (known.has(arg.text) && !found.has(arg.text)) (found.set(arg.text, where(call)), foundIn.set(arg.text, current));
        return;
      }
      if (!CALLEES.has(name)) return;
      // A const / union of literals resolves through the checker; a parameter is a wrapper passing its caller's name on.
      const t = checker.getTypeAtLocation(arg);
      const literals = t.isUnion() ? t.types : [t];
      if (literals.length && literals.every((x) => x.isStringLiteral())) {
        for (const x of literals) if (known.has(x.value) && !found.has(x.value)) (found.set(x.value, where(call)), foundIn.set(x.value, current));
        return;
      }
      const isParam = ts.isIdentifier(arg) && checker.getSymbolAtLocation(arg)?.declarations?.some((d) => ts.isParameter(d));
      if (!isParam && t.flags & ts.TypeFlags.StringLike) dynamic.push(where(call));
    };

    // A store's `withMethods` / `withComputed` factory runs when the store is created, but the members it returns run
    // only when something uses them: those are followed by reference, not by construction.
    const isLazyMembers = (n) =>
      ts.isCallExpression(n) && ts.isIdentifier(n.expression) && (n.expression.text === 'withMethods' || n.expression.text === 'withComputed');
    const visitFactory = (call, visit) => {
      for (const arg of call.arguments) {
        if (!ts.isArrowFunction(arg) && !ts.isFunctionExpression(arg)) {
          visit(arg);
          continue;
        }
        arg.parameters.forEach(visit);
        const body = arg.body;
        const deferred = (lit) => lit.properties.forEach((p) => (ts.isPropertyAssignment(p) || ts.isMethodDeclaration(p) || ts.isGetAccessor(p) || ts.isShorthandPropertyAssignment(p) ? undefined : visit(p)));
        // Helpers declared in the factory (`const save = async () => ...`) run when called, not when the store is made.
        const isHelper = (st) =>
          ts.isFunctionDeclaration(st) ||
          (ts.isVariableStatement(st) && st.declarationList.declarations.every((d) => d.initializer && (ts.isArrowFunction(d.initializer) || ts.isFunctionExpression(d.initializer))));
        const skipReturned = (x) => (ts.isParenthesizedExpression(x) ? skipReturned(x.expression) : ts.isObjectLiteralExpression(x) ? deferred(x) : visit(x));
        if (!ts.isBlock(body)) skipReturned(body);
        else for (const st of body.statements) ts.isReturnStatement(st) && st.expression ? skipReturned(st.expression) : isHelper(st) ? undefined : visit(st);
      }
    };

    for (const d of seeds) {
      current = null;
      follow(d, false);
    }
    while (queue.length) {
      const root = queue.pop();
      current = root;
      const visit = (n) => {
        if (ts.isObjectLiteralExpression(n) && lazyLiterals.has(n) && n !== root) {
          // Members of an object returned as an interface's implementation run when something calls them.
          for (const p of n.properties) if (!(ts.isPropertyAssignment(p) || ts.isMethodDeclaration(p) || ts.isShorthandPropertyAssignment(p))) visit(p);
          return;
        }
        if (isLazyMembers(n) && n !== root) {
          visit(n.expression);
          visitFactory(n, visit);
          return;
        }
        if (ts.isCallExpression(n)) {
          command(n);
          // inject(Class) / inject(TOKEN): what is injected is constructed.
          if (ts.isIdentifier(n.expression) && n.expression.text === 'inject' && n.arguments[0]) {
            const a = n.arguments[0];
            const id = ts.isIdentifier(a) ? a : ts.isPropertyAccessExpression(a) ? a.name : null;
            if (id) for (const d of declsOf(id)) follow(d, true);
          }
        } else if (ts.isNewExpression(n) && ts.isIdentifier(n.expression)) {
          for (const d of declsOf(n.expression)) follow(d, true);
        }
        if (ts.isIdentifier(n) && !ts.isDeclarationName?.(n)) {
          const parent = n.parent;
          const isDeclName = parent && 'name' in parent && parent.name === n && (ts.isPropertyDeclaration(parent) || ts.isMethodDeclaration(parent) || ts.isVariableDeclaration(parent) || ts.isParameter(parent) || ts.isFunctionDeclaration(parent) || ts.isClassDeclaration(parent));
          if (!isDeclName) for (const d of declsOf(n)) if (d !== root) follow(d, false);
        }
        ts.forEachChild(n, visit);
      };
      visit(root);
    }
    /** The chain of code that led to a command's call site (for explaining a surprising entry). */
    const chain = (command) => {
      const out = [];
      for (let n = foundIn.get(command) ?? null; n; n = via.get(n)) out.push(where(n));
      return out;
    };
    return { commands: found, dynamic, chain };
  }

  /** The entry nodes of a component class: all its members, its template-imported components and its providers. */
  function componentEntries(cls) {
    const out = [];
    const seenClass = new Set();
    const add = (c) => {
      if (seenClass.has(c)) return;
      seenClass.add(c);
      for (const m of c.members) out.push(m);
      for (const decl of templateReferences(c)) out.push({ decl });
      for (const dec of ts.getDecorators?.(c) ?? []) {
        const arg = ts.isCallExpression(dec.expression) ? dec.expression.arguments[0] : null;
        if (!arg || !ts.isObjectLiteralExpression(arg)) continue;
        for (const p of arg.properties) {
          if (!ts.isPropertyAssignment(p)) continue;
          const key = p.name.getText();
          if (key === 'providers' || key === 'viewProviders' || key === 'hostDirectives') out.push(p.initializer);
          if (key === 'imports' && ts.isArrayLiteralExpression(p.initializer)) {
            for (const el of p.initializer.elements) {
              if (!ts.isIdentifier(el)) continue;
              for (const d of declsOf(el)) if (ts.isClassDeclaration(d)) add(d);
            }
          }
        }
      }
    };
    add(cls);
    return out;
  }

  /** Template and host-binding text of a component: inline, or the `templateUrl` file. */
  function templateText(cls) {
    let text = '';
    for (const dec of ts.getDecorators?.(cls) ?? []) {
      const arg = ts.isCallExpression(dec.expression) ? dec.expression.arguments[0] : null;
      if (!arg || !ts.isObjectLiteralExpression(arg)) continue;
      for (const p of arg.properties) {
        if (!ts.isPropertyAssignment(p)) continue;
        const key = p.name.getText();
        if (key === 'template' || key === 'host') text += `\n${p.initializer.getText()}`;
        if (key === 'templateUrl' && ts.isStringLiteralLike(p.initializer)) {
          try {
            text += `\n${readFileSync(resolve(dirname(cls.getSourceFile().fileName), p.initializer.text), 'utf8')}`;
          } catch {
            /* missing template file: the compiler reports it */
          }
        }
      }
    }
    return text;
  }

  /**
   * The declarations a component's template reaches through its members: `store.resetSection(...)`, `a.b().c`. Each
   * chain is walked through the types (a call goes to the return type), and every property on it is a reference.
   */
  function templateReferences(cls) {
    const text = templateText(cls);
    if (!text) return [];
    const members = new Map();
    for (const m of cls.members) if (m.name && ts.isIdentifier(m.name)) members.set(m.name.text, m);
    const out = [];
    const ident = /[A-Za-z_$][\w$]*/y;
    for (let i = 0; i < text.length; i++) {
      if (!/[A-Za-z_$]/.test(text[i]) || /[\w$.]/.test(text[i - 1] ?? ' ')) continue;
      ident.lastIndex = i;
      const head = ident.exec(text)?.[0];
      if (!head) continue;
      const decl = members.get(head);
      let at = i + head.length;
      if (!decl) {
        i = at - 1;
        continue;
      }
      let type = checker.getTypeAtLocation(decl);
      const step = () => {
        const sigs = type.getCallSignatures?.() ?? [];
        if (sigs.length) type = sigs[0].getReturnType();
      };
      while (at < text.length) {
        while (text[at] === '!' || text[at] === ' ') at++;
        if (text[at] === '(') {
          let depth = 0;
          for (; at < text.length; at++) {
            if (text[at] === '(') depth++;
            else if (text[at] === ')' && --depth === 0) {
              at++;
              break;
            }
          }
          step();
          continue;
        }
        const dot = /^\s*\??\.\s*([A-Za-z_$][\w$]*)/.exec(text.slice(at, at + 200));
        if (!dot) break;
        const prop = checker.getNonNullableType(type).getProperty(dot[1]);
        for (const d of prop?.declarations ?? []) if (own(d.getSourceFile())) out.push(d);
        type = prop ? checker.getTypeOfSymbol(prop) : type;
        at += dot[0].length;
      }
      i = Math.max(i, at - 1);
    }
    return out;
  }

  return { program, checker, reach, componentEntries, declsOf, own };
}

/** class declaration -> the classes that extend or implement it. */
function indexImplementers(program, checker, own) {
  const out = new Map();
  for (const sf of program.getSourceFiles()) {
    if (!own(sf)) continue;
    const visit = (n) => {
      if (ts.isClassDeclaration(n)) {
        for (const h of n.heritageClauses ?? []) {
          for (const t of h.types) {
            let s = checker.getSymbolAtLocation(t.expression);
            if (s && s.flags & ts.SymbolFlags.Alias) s = checker.getAliasedSymbol(s);
            for (const d of s?.declarations ?? []) {
              if (ts.isClassDeclaration(d) || ts.isInterfaceDeclaration(d)) {
                if (!out.has(d)) out.set(d, []);
                out.get(d).push(n);
              }
            }
          }
        }
      }
      ts.forEachChild(n, visit);
    };
    visit(sf);
  }
  return out;
}

/**
 * Factories that return an object literal as an interface (`function tauriIo(): DialogIo { return { open: ... } }`):
 * interface -> the literals implementing it, and the set of those literals (their members are followed by use).
 */
function indexObjectImplementations(program, checker, own) {
  const objectImpls = new Map();
  const lazyLiterals = new Set();
  const literalOf = (x) => {
    while (x && ts.isParenthesizedExpression(x)) x = x.expression;
    return x && ts.isObjectLiteralExpression(x) ? x : null;
  };
  for (const sf of program.getSourceFiles()) {
    if (!own(sf)) continue;
    const visit = (n) => {
      if ((ts.isFunctionDeclaration(n) || ts.isArrowFunction(n) || ts.isFunctionExpression(n) || ts.isMethodDeclaration(n)) && n.type && ts.isTypeReferenceNode(n.type) && n.body) {
        let s = checker.getSymbolAtLocation(n.type.typeName);
        if (s && s.flags & ts.SymbolFlags.Alias) s = checker.getAliasedSymbol(s);
        const target = (s?.declarations ?? []).find(ts.isInterfaceDeclaration);
        if (target) {
          const lits = [];
          if (ts.isBlock(n.body)) {
            const find = (m) => {
              if (ts.isReturnStatement(m)) {
                const l = literalOf(m.expression);
                if (l) lits.push(l);
              } else if (!ts.isFunctionLike(m)) ts.forEachChild(m, find);
            };
            n.body.statements.forEach(find);
          } else {
            const l = literalOf(n.body);
            if (l) lits.push(l);
          }
          for (const l of lits) {
            lazyLiterals.add(l);
            if (!objectImpls.has(target)) objectImpls.set(target, []);
            objectImpls.get(target).push(l);
          }
        }
      }
      ts.forEachChild(n, visit);
    };
    visit(sf);
  }
  return { objectImpls, lazyLiterals };
}

/** Declared registry entries: id, entry class (from the lazy import) and the declared command list, read from the source. */
export function readRegistry(analysis, registryFile) {
  const sf = analysis.program.getSourceFile(registryFile);
  if (!sf) throw new Error(`${registryFile} is not part of the program`);
  const out = [];
  const text = (n) => (n && ts.isStringLiteralLike(n) ? n.text : null);
  const visit = (n) => {
    if (ts.isCallExpression(n) && ts.isIdentifier(n.expression) && n.expression.text === 'defineDialog' && n.arguments[0] && ts.isObjectLiteralExpression(n.arguments[0])) {
      const entry = { id: '', component: null, commands: [], line: sf.getLineAndCharacterOfPosition(n.getStart()).line + 1 };
      for (const p of n.arguments[0].properties) {
        if (!ts.isPropertyAssignment(p)) continue;
        const key = p.name.getText();
        if (key === 'id') entry.id = text(p.initializer) ?? '';
        if (key === 'commands' && ts.isArrayLiteralExpression(p.initializer)) entry.commands = p.initializer.elements.map(text).filter((x) => x !== null);
        if (key === 'load') {
          // () => import('..').then((m) => m.Class)
          let cls = null;
          const find = (x) => {
            if (ts.isPropertyAccessExpression(x) && ts.isIdentifier(x.expression) && x.expression.text === 'm') cls = x.name;
            ts.forEachChild(x, find);
          };
          find(p.initializer);
          if (cls) entry.component = cls;
        }
      }
      out.push(entry);
    }
    ts.forEachChild(n, visit);
  };
  visit(sf);
  return out;
}

/** The string elements of the exported array constant `name` in `file`. */
export function readStringArray(analysis, file, name) {
  const sf = analysis.program.getSourceFile(file);
  let out = null;
  const visit = (n) => {
    if (ts.isVariableDeclaration(n) && n.name.getText() === name && n.initializer && ts.isArrayLiteralExpression(n.initializer)) out = n.initializer.elements.filter(ts.isStringLiteralLike).map((e) => e.text);
    ts.forEachChild(n, visit);
  };
  if (sf) visit(sf);
  return out;
}

export { dirname, join, resolve };
