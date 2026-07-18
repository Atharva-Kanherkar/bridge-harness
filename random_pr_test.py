"""A tiny random-number example for testing the PR workflow."""

from random import Random


def roll_dice(seed: int, count: int = 5) -> list[int]:
    """Return a reproducible handful of six-sided dice rolls."""
    generator = Random(seed)
    return [generator.randint(1, 6) for _ in range(count)]


if __name__ == "__main__":
    print(roll_dice(seed=42))
