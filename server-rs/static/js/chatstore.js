// IndexedDB persistence for Studio Agent conversations. Records hold the
// full message list (OpenAI format, view_screen images included) plus
// metadata (title, timestamps, model, design, token usage) so the History
// view can list and reopen them. Everything stays in this browser.

const DB_NAME = "yello-studio";
const STORE = "conversations";

let dbPromise = null;

function openDb() {
  if (!dbPromise) {
    dbPromise = new Promise((resolve, reject) => {
      const request = indexedDB.open(DB_NAME, 1);
      request.onupgradeneeded = () => {
        const db = request.result;
        if (!db.objectStoreNames.contains(STORE)) {
          db.createObjectStore(STORE, { keyPath: "id" });
        }
      };
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => reject(request.error);
    });
  }
  return dbPromise;
}

async function run(mode, action) {
  const db = await openDb();
  return new Promise((resolve, reject) => {
    const transaction = db.transaction(STORE, mode);
    const store = transaction.objectStore(STORE);
    let result;
    const request = action(store);
    if (request) request.onsuccess = () => { result = request.result; };
    transaction.oncomplete = () => resolve(result);
    transaction.onerror = () => reject(transaction.error);
  });
}

/** Insert or replace a conversation record. */
export async function saveChat(record) {
  await run("readwrite", (store) => store.put(record));
}

/** All conversation metadata (messages stripped), newest first. */
export async function listChats() {
  const all = (await run("readonly", (store) => store.getAll())) || [];
  return all
    .map(({ messages, ...meta }) => ({ ...meta, messageCount: (messages || []).length }))
    .sort((a, b) => b.updatedAt - a.updatedAt);
}

/** One full conversation record, or null. */
export async function getChat(id) {
  return (await run("readonly", (store) => store.get(id))) || null;
}

export async function deleteChat(id) {
  await run("readwrite", (store) => store.delete(id));
}
