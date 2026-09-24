FROM python:3.11-bookworm

RUN python -m pip install --no-cache-dir ruamel.yaml==0.18.16 PyYAML==6.0.3

WORKDIR /workspace
CMD ["python", "-m", "unittest", "discover", "-s", "template/tests", "-v"]
