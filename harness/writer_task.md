You are a writer agent for the rctest7-92b4b8ab test run. Your task: insert 5 orders into rctest7_92b4b8ab_orders table. Columns: id (auto-generated), writer (your session_id), amount (random $10-$100), created_at (ISO timestamp). Insert one at a time, ~2 seconds apart. After each insert:
1. Write the order to the database
2. Emit a change notification to state scope rctest7_92b4b8ab_changes with key rctest7_92b4b8ab_change
3. Wait for the reactor to process
4. Repeat until 5 orders done, then mark yourself done in rctest7_92b4b8ab_writers table
Make sure to be idempotent in your approach. Use harness::spawn for any sub-tasks you need. Record your session_id in each order so we can track which writer inserted what.