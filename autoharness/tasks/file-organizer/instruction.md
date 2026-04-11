In the current directory you will find these files:

```
report.pdf
photo.jpg
notes.txt
data.csv
image.png
readme.txt
budget.csv
```

Write a script called `organize.sh` that, when run, organizes these files into subdirectories by extension:

```
documents/report.pdf
images/photo.jpg
images/image.png
text/notes.txt
text/readme.txt
data/data.csv
data/budget.csv
```

Mapping:
- `.pdf` -> `documents/`
- `.jpg`, `.png` -> `images/`
- `.txt` -> `text/`
- `.csv` -> `data/`

Create the directories if they don't exist. Move the files (not copy). Then run the script.
