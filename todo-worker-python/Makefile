.PHONY: build run dev clean

build:
	docker build -t todo-worker-python .

run:
	docker run --rm -e III_URL todo-worker-python

dev:
	pip install -e . && todo-worker

clean:
	rm -rf .venv __pycache__ src/__pycache__
