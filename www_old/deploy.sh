#!/bin/bash
ssh rocky@37.187.194.63 'bash -c "cd apps/solisoft/solidb/www && git pull && sh restart.sh"'
