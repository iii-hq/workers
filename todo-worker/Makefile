.PHONY: build run dev clean

build:
	docker build -t todo-worker .

run:
	docker run --rm -e III_URL todo-worker

dev:
	npm run dev

clean:
	rm -rf dist node_modules
