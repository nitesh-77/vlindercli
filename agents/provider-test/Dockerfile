FROM python:3.12-alpine
RUN pip install --no-cache-dir openai
COPY test_driver.py /test_driver.py
EXPOSE 8080
ENTRYPOINT ["python3", "/test_driver.py"]
