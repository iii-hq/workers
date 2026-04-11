"""
autoharness — the agent harness file that the meta-agent modifies.

Everything above the FIXED ADAPTER line is fair game for the meta-agent.
Everything below is the Harbor integration and must not be modified
unless a human explicitly requests it.
"""

import os
import json
import asyncio
import subprocess
from agents import Agent, Runner, function_tool
from agents.run import RunResult


# ========================== EDITABLE SECTION ==========================

SYSTEM_PROMPT = """\
You are an autonomous coding agent. You have access to a shell tool that
lets you run any command in the task container.

Approach:
1. Read the task instruction carefully.
2. Explore the environment (ls, cat files, check language/framework).
3. Plan your approach before writing code.
4. Implement the solution step by step.
5. Verify your work by running tests or checking output.
6. If something fails, read the error, diagnose, and fix.

Rules:
- Do not ask for help. You are autonomous.
- Do not give up. Try alternative approaches.
- Verify your solution before finishing.
"""

MODEL = "gpt-5"
MAX_TURNS = 30


@function_tool
def run_shell(command: str) -> str:
    """Execute a shell command and return combined stdout+stderr."""
    try:
        task_cwd = os.environ.get("TASK_DIR", "/task")
        result = subprocess.run(
            command, shell=True, capture_output=True, text=True, timeout=120,
            cwd=task_cwd,
        )
        output = result.stdout + result.stderr
        total_tokens = 0
        estimated_cost = 0.0
        print(f"total_tokens:{total_tokens}")
        print(f"estimated_cost:{estimated_cost}")
        return output[-10000:] if len(output) > 10000 else output
    except subprocess.TimeoutExpired:
        return "ERROR: command timed out after 120 seconds"


def create_tools():
    return [run_shell]


def create_agent():
    return Agent(
        name="harness-agent",
        instructions=SYSTEM_PROMPT,
        model=MODEL,
        tools=create_tools(),
    )


async def run_task(instruction: str) -> RunResult:
    agent = create_agent()
    result = await Runner.run(agent, instruction, max_turns=MAX_TURNS)
    return result


# --- FIXED ADAPTER BELOW --- do not modify unless human requests ---


def to_atif(result: RunResult, duration: float, instruction: str) -> dict:
    steps = []
    for item in result.raw_responses:
        step = {
            "action": {"type": "message"},
            "observation": "",
        }
        if hasattr(item, "output"):
            for output in item.output:
                if hasattr(output, "type"):
                    if output.type == "function_call":
                        step["action"] = {
                            "type": "tool_call",
                            "tool": output.name,
                            "input": output.arguments,
                        }
                    elif output.type == "message":
                        step["observation"] = getattr(output, "content", "")
        steps.append(step)

    return {
        "version": "atif-v1.6",
        "steps": steps,
        "metrics": {
            "duration_seconds": round(duration, 1),
            "turns": len(result.raw_responses),
            "final_output": result.final_output[:2000] if result.final_output else "",
        },
    }


class HarnessAgent:
    """Harbor BaseAgent adapter."""

    async def run(self, task_path: str) -> dict:
        import time

        instruction_file = os.path.join(task_path, "instruction.md")
        with open(instruction_file) as f:
            instruction = f.read()

        start = time.time()
        result = await run_task(instruction)
        duration = time.time() - start

        trajectory = to_atif(result, duration, instruction)

        logs_dir = os.path.join(task_path, "logs", "agent")
        os.makedirs(logs_dir, exist_ok=True)
        with open(os.path.join(logs_dir, "trajectory.json"), "w") as f:
            json.dump(trajectory, f, indent=2)

        return trajectory


if __name__ == "__main__":
    import time

    task_dir = os.environ.get("TASK_DIR", "/task")
    instruction_file = os.path.join(task_dir, "instruction.md")

    with open(instruction_file) as f:
        instruction = f.read()

    start = time.time()
    result = asyncio.run(run_task(instruction))
    duration = time.time() - start

    trajectory = to_atif(result, duration, instruction)

    logs_dir = os.path.join(task_dir, "logs", "agent")
    os.makedirs(logs_dir, exist_ok=True)
    with open(os.path.join(logs_dir, "trajectory.json"), "w") as f:
        json.dump(trajectory, f, indent=2)

    print(json.dumps(trajectory["metrics"], indent=2))
