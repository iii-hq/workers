#!/usr/bin/env python3
import sqlite3
import time
import datetime
import random

conn = sqlite3.connect('./data/iii.db')
cursor = conn.cursor()\n
writer_id = 'writer-1'  # Will be overridden by task

for i in range(5):
    amount = round(random.uniform(100, 150), 2)
    created_at = datetime.datetime.now().isoformat()
    
    cursor.execute(
        'INSERT INTO rctest7_a7b9_orders (writer, amount, created_at) VALUES (?, ?, ?)',
        (writer_id, amount, created_at)
    )
    conn.commit()
    
    # Emit change signal
    cursor.execute(
        'INSERT INTO rctest7_a7b9_writers (writer_id, status) VALUES (?, ?) '
        'ON CONFLICT(writer_id) DO UPDATE SET status = ?',
        (writer_id, 'in_progress', 'in_progress')
    )
    conn.commit()
    
    print(f'Inserted order {i+1} for {writer_id}: amount={amount}')
    time.sleep(2)

# Mark done
cursor.execute(
    'INSERT INTO rctest7_a7b9_writers (writer_id, status) VALUES (?, ?) '
    'ON CONFLICT(writer_id) DO UPDATE SET status = ?',
    (writer_id, 'done', 'done')
)
conn.commit()

print(f'Writer {writer_id} complete')
conn.close()