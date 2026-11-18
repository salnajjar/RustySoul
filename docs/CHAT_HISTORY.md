# Saved chats and room messages

Private chats and room messages stay in SQLite until you delete them. Restarting RustySoul, closing a chat, or leaving a room keeps the saved messages. Reopening a conversation loads its most recent messages; use **Load older messages** to continue back through its history. New arrivals appear alongside the existing conversation.

The Messages page lists saved private conversations. **Close chat** closes the view and keeps its history. The Rooms page also has **Saved room history**, which lets you read previous conversations while disconnected or after leaving a room. You must join a room to send new room messages.

Choose **Delete history** in a conversation header, then **Delete history permanently** to remove all its saved messages, including older pages. Cancel leaves everything in place. Deletion applies only to that selected private chat or room. New messages received or sent afterwards start a new saved history. Existing backup files retain the messages they contained when created; restoring an older backup can bring that history back.

History loads in small pages to keep the browser and daemon lightweight. The live event stream keeps a 500-message preview across conversations; this preview does not limit or erase the permanent database history. There is no automatic message-history expiration. Messages already purged by older releases are available only if retained in an existing backup.
