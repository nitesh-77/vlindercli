FROM python:3.12-alpine
COPY --from=public.ecr.aws/awsguru/aws-lambda-adapter:0.9.0 /lambda-adapter /opt/extensions/lambda-adapter
ENV AWS_LWA_PASS_THROUGH_PATH="/invoke"
ENV AWS_LWA_READINESS_CHECK_PATH="/health"
COPY echo_server.py /echo_server.py
EXPOSE 8080
ENTRYPOINT ["python3", "/echo_server.py"]
