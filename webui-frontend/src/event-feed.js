// Message IDs are local database IDs shared by the HTTP snapshot and WebSocket.
// Text and timestamps cannot identify an event: two real messages may be identical.
export function messageEventKey(event) {
  const payload = event.payload ?? event
  return event.kind === 'MessageReceived' && payload.msg_id != null
    ? `message:${payload.msg_id}`
    : null
}

export function mergeEventRows(entries, limit = Infinity) {
  const rows = [], positions = new Map()
  for (const entry of entries) {
    const key = messageEventKey(entry) ?? (entry.id != null ? `event:${entry.id}` : null)
    if (key != null && positions.has(key)) {
      const index = positions.get(key)
      // Keep the persisted ID for pagination and selection when the live copy
      // arrived first. Its position still follows the incoming newest-first order.
      if (rows[index].id == null && entry.id != null) rows[index] = entry
      continue
    }
    if (key != null) positions.set(key, rows.length)
    rows.push(entry)
  }
  return rows.slice(0, limit)
}
