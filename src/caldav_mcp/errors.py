from typing import Any


class CalendarError(Exception):
    def __init__(self, code: str, message: str, **details: Any) -> None:
        super().__init__(message)
        self.code = code
        self.details = details

    def result(self) -> dict[str, Any]:
        return {"code": self.code, "message": str(self), **self.details}
