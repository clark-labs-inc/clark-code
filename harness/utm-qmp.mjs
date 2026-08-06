import net from "node:net";

const SHIFTED = new Map([
  ["!", "1"],
  ["@", "2"],
  ["#", "3"],
  ["$", "4"],
  ["%", "5"],
  ["^", "6"],
  ["&", "7"],
  ["*", "8"],
  ["(", "9"],
  [")", "0"],
  ["_", "minus"],
  ["+", "equal"],
  ["{", "bracket_left"],
  ["}", "bracket_right"],
  ["|", "backslash"],
  [":", "semicolon"],
  ['"', "apostrophe"],
  ["~", "grave_accent"],
  ["<", "comma"],
  [">", "dot"],
  ["?", "slash"],
]);

const PLAIN = new Map([
  [" ", "spc"],
  ["-", "minus"],
  ["=", "equal"],
  ["[", "bracket_left"],
  ["]", "bracket_right"],
  ["\\", "backslash"],
  [";", "semicolon"],
  ["'", "apostrophe"],
  ["`", "grave_accent"],
  [",", "comma"],
  [".", "dot"],
  ["/", "slash"],
  ["\n", "ret"],
  ["\r", "ret"],
  ["\t", "tab"],
]);

function delay(milliseconds) {
  if (milliseconds <= 0) return Promise.resolve();
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

export function encodeTextToQCodes(text) {
  const encoded = [];
  for (const character of String(text)) {
    if (/^[a-z0-9]$/.test(character)) {
      encoded.push([character]);
    } else if (/^[A-Z]$/.test(character)) {
      encoded.push(["shift", character.toLowerCase()]);
    } else if (PLAIN.has(character)) {
      encoded.push([PLAIN.get(character)]);
    } else if (SHIFTED.has(character)) {
      encoded.push(["shift", SHIFTED.get(character)]);
    } else {
      throw new Error(`QMP keyboard transport does not support character U+${character.codePointAt(0).toString(16).toUpperCase()}`);
    }
  }
  return encoded;
}

export class QmpClient {
  constructor({ host = "127.0.0.1", port, timeoutMs = 5_000 } = {}) {
    if (!Number.isInteger(port) || port < 1 || port > 65_535) {
      throw new Error("QMP port must be an integer from 1 through 65535");
    }
    this.host = host;
    this.port = port;
    this.timeoutMs = timeoutMs;
    this.socket = null;
    this.buffer = "";
    this.pending = new Map();
    this.nextId = 1;
    this.greeting = null;
  }

  async connect() {
    if (this.socket) return;
    await new Promise((resolve, reject) => {
      const socket = net.createConnection({ host: this.host, port: this.port });
      const timer = setTimeout(() => {
        socket.destroy();
        reject(new Error(`QMP connection timed out on ${this.host}:${this.port}`));
      }, this.timeoutMs);
      socket.setEncoding("utf8");
      socket.on("data", (chunk) => this.#onData(chunk));
      socket.once("error", (error) => {
        clearTimeout(timer);
        reject(error);
      });
      socket.once("connect", async () => {
        this.socket = socket;
        try {
          await this.#waitForGreeting();
          await this.execute("qmp_capabilities");
          clearTimeout(timer);
          resolve();
        } catch (error) {
          clearTimeout(timer);
          socket.destroy();
          reject(error);
        }
      });
      socket.once("close", () => {
        const error = new Error("QMP connection closed");
        for (const waiter of this.pending.values()) waiter.reject(error);
        this.pending.clear();
        this.socket = null;
      });
    });
  }

  async execute(command, args = undefined) {
    if (!this.socket) throw new Error("QMP client is not connected");
    const id = this.nextId++;
    const payload = { execute: command, id };
    if (args !== undefined) payload.arguments = args;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`QMP command ${command} timed out`));
      }, this.timeoutMs);
      this.pending.set(id, {
        resolve: (value) => {
          clearTimeout(timer);
          resolve(value);
        },
        reject: (error) => {
          clearTimeout(timer);
          reject(error);
        },
      });
      this.socket.write(`${JSON.stringify(payload)}\r\n`);
    });
  }

  async sendChord(qcodes, holdMs = 35) {
    if (!Array.isArray(qcodes) || qcodes.length === 0) {
      throw new Error("QMP chord must contain at least one qcode");
    }
    await this.execute("send-key", {
      keys: qcodes.map((code) => ({ type: "qcode", data: code })),
      "hold-time": holdMs,
    });
  }

  async typeText(text, { interKeyMs = 55 } = {}) {
    for (const qcodes of encodeTextToQCodes(text)) {
      await this.sendChord(qcodes);
      await delay(interKeyMs);
    }
  }

  async openWindowsRunAndExecute(command, { settleMs = 500 } = {}) {
    await this.sendChord(["meta_l", "r"]);
    await delay(settleMs);
    await this.sendChord(["ctrl", "a"]);
    await this.sendChord(["backspace"]);
    await this.typeText(command);
    await this.sendChord(["ret"]);
  }

  close() {
    this.socket?.end();
    this.socket = null;
  }

  #onData(chunk) {
    this.buffer += chunk;
    while (true) {
      const newline = this.buffer.indexOf("\n");
      if (newline < 0) break;
      const line = this.buffer.slice(0, newline).trim();
      this.buffer = this.buffer.slice(newline + 1);
      if (!line) continue;
      let message;
      try {
        message = JSON.parse(line);
      } catch {
        continue;
      }
      if (message.QMP && !this.greeting) {
        this.greeting = message;
        continue;
      }
      if (message.id === undefined) continue;
      const waiter = this.pending.get(message.id);
      if (!waiter) continue;
      this.pending.delete(message.id);
      if (message.error) {
        waiter.reject(new Error(message.error.desc || JSON.stringify(message.error)));
      } else {
        waiter.resolve(message.return);
      }
    }
  }

  async #waitForGreeting() {
    const deadline = Date.now() + this.timeoutMs;
    while (!this.greeting) {
      if (Date.now() >= deadline) throw new Error("QMP greeting timed out");
      await delay(10);
    }
  }
}
